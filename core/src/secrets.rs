//! Short-lived pinentry buffers and locked session secrets. No Debug implementation.
use std::{io, path::Path, process::Stdio, ptr::NonNull, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};
use zeroize::{Zeroize, Zeroizing};

const PAGE: usize = 4096;
pub type SecureBytes = Zeroizing<Vec<u8>>;

pub struct SessionSecret {
    memory: NonNull<u8>,
    length: usize,
}
// The allocation is immutable after construction and only released by the final owner.
unsafe impl Send for SessionSecret {}
unsafe impl Sync for SessionSecret {}

impl SessionSecret {
    pub fn new(bytes: &[u8]) -> Result<Arc<Self>, &'static str> {
        if bytes.is_empty()
            || bytes.len() >= PAGE
            || bytes.contains(&0)
            || bytes.contains(&b'\n')
            || bytes.contains(&b'\r')
        {
            return Err("invalid_password");
        }
        let memory = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                PAGE,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if memory == libc::MAP_FAILED {
            return Err("secret_memory_failed");
        }
        if unsafe { libc::mlock(memory, PAGE) } != 0
            || unsafe { libc::madvise(memory, PAGE, libc::MADV_DONTDUMP) } != 0
        {
            unsafe {
                libc::munmap(memory, PAGE);
            }
            return Err("secret_memory_lock_failed");
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), memory.cast::<u8>(), bytes.len());
        }
        Ok(Arc::new(Self {
            memory: NonNull::new(memory.cast()).ok_or("secret_memory_failed")?,
            length: bytes.len(),
        }))
    }

    pub(crate) fn expose(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.memory.as_ptr(), self.length) }
    }
}

impl Drop for SessionSecret {
    fn drop(&mut self) {
        unsafe {
            std::slice::from_raw_parts_mut(self.memory.as_ptr(), PAGE).zeroize();
            libc::munlock(self.memory.as_ptr().cast(), PAGE);
            libc::munmap(self.memory.as_ptr().cast(), PAGE);
        }
    }
}

pub fn disable_dumps() -> Result<(), &'static str> {
    let limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) } != 0
        || unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0) } != 0
    {
        return Err("cannot_disable_dumps");
    }
    Ok(())
}

pub(crate) async fn secure_read<R: AsyncRead + Unpin>(
    mut source: R,
    limit: usize,
) -> Result<SecureBytes, &'static str> {
    let mut buffer = Zeroizing::new(vec![0u8; limit + 1]);
    let mut count = 0;
    loop {
        let n = source
            .read(&mut buffer[count..])
            .await
            .map_err(|_| "process_io")?;
        count += n;
        if count > limit {
            return Err("output_limit");
        }
        if n == 0 {
            break;
        }
    }
    buffer.truncate(count);
    Ok(buffer)
}

async fn line<R: AsyncRead + Unpin>(source: &mut R) -> Result<SecureBytes, &'static str> {
    let mut bytes = Zeroizing::new(Vec::with_capacity(16384));
    loop {
        if bytes.len() == 16384 {
            return Err("pinentry_protocol");
        }
        let byte = source.read_u8().await.map_err(|_| "pinentry_protocol")?;
        if byte == b'\n' {
            break;
        }
        bytes.push(byte);
    }
    Ok(bytes)
}

fn decode_data(input: &[u8]) -> Result<SecureBytes, &'static str> {
    let mut result = Zeroizing::new(Vec::with_capacity(PAGE));
    let mut index = 0;
    while index < input.len() {
        if result.len() >= PAGE - 1 {
            return Err("invalid_password");
        }
        if input[index] == b'%' {
            let digits = input.get(index + 1..index + 3).ok_or("pinentry_protocol")?;
            let hex = std::str::from_utf8(digits).map_err(|_| "pinentry_protocol")?;
            result.push(u8::from_str_radix(hex, 16).map_err(|_| "pinentry_protocol")?);
            index += 3;
        } else {
            result.push(input[index]);
            index += 1;
        }
    }
    Ok(result)
}

pub(crate) fn desktop_environment(command: &mut Command) {
    for key in [
        "HOME",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "LANG",
        "LC_CTYPE",
        "XDG_CURRENT_DESKTOP",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.env("PATH", "/usr/local/bin:/usr/bin:/bin");
}

async fn prompt(
    description: &'static str,
    prompt: &'static str,
    cancelled: &'static str,
) -> Result<Arc<SessionSecret>, &'static str> {
    let program = if Path::new("/usr/bin/pinentry-qt").is_file() {
        "/usr/bin/pinentry-qt"
    } else {
        "/usr/bin/pinentry-gnome3"
    };
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    desktop_environment(&mut command);
    let mut child = command.spawn().map_err(|_| "pinentry_unavailable")?;
    let mut input = child.stdin.take().ok_or("pinentry_protocol")?;
    let mut output = child.stdout.take().ok_or("pinentry_protocol")?;
    let operation = async {
        if !line(&mut output).await?.starts_with(b"OK") {
            return Err("pinentry_protocol");
        }
        for request in ["SETTITLE Hetzner Drive", description, prompt] {
            input
                .write_all(request.as_bytes())
                .await
                .map_err(|_| "pinentry_protocol")?;
            input
                .write_all(b"\n")
                .await
                .map_err(|_| "pinentry_protocol")?;
            if !line(&mut output).await?.starts_with(b"OK") {
                return Err("pinentry_protocol");
            }
        }
        // A fresh pinentry has no external cache permission. Never send
        // allow-external-password-cache or SETKEYINFO (needed for persistent caching).
        input
            .write_all(b"GETPIN\n")
            .await
            .map_err(|_| "pinentry_protocol")?;
        let mut password = Zeroizing::new(Vec::with_capacity(PAGE));
        loop {
            let response = line(&mut output).await?;
            if response.starts_with(b"ERR") {
                return Err(cancelled);
            }
            if let Some(data) = response.strip_prefix(b"D ") {
                let decoded = decode_data(data)?;
                if password.len() + decoded.len() >= PAGE {
                    return Err("invalid_password");
                }
                password.extend_from_slice(&decoded);
            } else if response.starts_with(b"OK") {
                return SessionSecret::new(&password);
            } else if !response.starts_with(b"S ") {
                return Err("pinentry_protocol");
            }
        }
    };
    let result = tokio::time::timeout(Duration::from_secs(180), operation)
        .await
        .unwrap_or(Err("unlock_timeout"));
    let _ = child.kill().await;
    let _ = child.wait().await;
    result
}

pub async fn prompt_password() -> Result<Arc<SessionSecret>, &'static str> {
    prompt(
        "SETDESC Enter the password protecting rclone.conf / Introdu parola care protejeaza rclone.conf. Do not enter the crypt password or salt / Nu introduce parola crypt sau salt-ul.",
        "SETPROMPT rclone.conf password / Parola rclone.conf:",
        "unlock_cancelled",
    )
    .await
}

pub async fn prompt_restic_password(
    confirmation: bool,
) -> Result<Arc<SessionSecret>, &'static str> {
    let (description, label) = if confirmation {
        (
            "SETDESC Confirm the NEW test restic repository password / Confirma parola NOULUI repository restic de test. This is not the rclone.conf or crypt password/salt / Nu este parola rclone.conf sau parola/salt-ul crypt.",
            "SETPROMPT Confirm restic password / Confirma parola restic:",
        )
    } else {
        (
            "SETDESC Enter the SEPARATE test restic repository password / Introdu parola SEPARATA a repository-ului restic de test. This is not the rclone.conf or crypt password/salt / Nu este parola rclone.conf sau parola/salt-ul crypt.",
            "SETPROMPT Restic repository password / Parola repository restic:",
        )
    };
    prompt(description, label, "backup_unlock_cancelled").await
}

pub async fn confirm_backup_action(restore: bool, selective: bool) -> Result<(), &'static str> {
    let program = if Path::new("/usr/bin/pinentry-qt").is_file() {
        "/usr/bin/pinentry-qt"
    } else {
        "/usr/bin/pinentry-gnome3"
    };
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    desktop_environment(&mut command);
    let mut child = command.spawn().map_err(|_| "pinentry_unavailable")?;
    let mut input = child.stdin.take().ok_or("pinentry_protocol")?;
    let mut output = child.stdout.take().ok_or("pinentry_protocol")?;
    let description = if selective {
        "SETDESC Confirm restore of the SELECTED file or directory into a NEW local directory / Confirma restaurarea fisierului sau directorului SELECTAT intr-un director local NOU. Existing files are not overwritten or deleted / Fisierele existente nu sunt suprascrise sau sterse."
    } else if restore {
        "SETDESC Confirm restore into a NEW local directory / Confirma restaurarea intr-un director local NOU. Existing files are not overwritten or deleted / Fisierele existente nu sunt suprascrise sau sterse."
    } else {
        "SETDESC Confirm backup to the disposable restic repository / Confirma backupul in repository-ul restic disposable. File contents will be read and uploaded encrypted / Continutul fisierelor va fi citit si incarcat criptat."
    };
    let operation = async {
        if !line(&mut output).await?.starts_with(b"OK") {
            return Err("pinentry_protocol");
        }
        for request in [
            "SETTITLE Hetzner Drive",
            description,
            if restore {
                "SETOK Restore / Restaureaza"
            } else {
                "SETOK Start backup / Porneste backup"
            },
            "SETCANCEL Cancel / Anuleaza",
        ] {
            input
                .write_all(request.as_bytes())
                .await
                .map_err(|_| "pinentry_protocol")?;
            input
                .write_all(b"\n")
                .await
                .map_err(|_| "pinentry_protocol")?;
            if !line(&mut output).await?.starts_with(b"OK") {
                return Err("pinentry_protocol");
            }
        }
        input
            .write_all(b"CONFIRM\n")
            .await
            .map_err(|_| "pinentry_protocol")?;
        if line(&mut output).await?.starts_with(b"OK") {
            Ok(())
        } else {
            Err("backup_confirmation_cancelled")
        }
    };
    let result = tokio::time::timeout(Duration::from_secs(180), operation)
        .await
        .unwrap_or(Err("backup_confirmation_timeout"));
    let _ = child.kill().await;
    let _ = child.wait().await;
    result
}

pub(crate) fn secrets_equal(left: &SessionSecret, right: &SessionSecret) -> bool {
    let left = left.expose();
    let right = right.expose();
    let mut difference = left.len() ^ right.len();
    let length = left.len().max(right.len());
    for index in 0..length {
        difference |= usize::from(
            left.get(index).copied().unwrap_or_default()
                ^ right.get(index).copied().unwrap_or_default(),
        );
    }
    difference == 0
}

pub(crate) fn inherit_fd(command: &mut Command, fd: i32) {
    unsafe {
        command.pre_exec(move || {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

pub(crate) fn random_name() -> Result<String, &'static str> {
    let mut bytes = [0u8; 16];
    if unsafe { libc::getrandom(bytes.as_mut_ptr().cast(), bytes.len(), 0) } != bytes.len() as isize
    {
        return Err("random_unavailable");
    }
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

pub(crate) fn trusted_runtime() -> Result<std::path::PathBuf, &'static str> {
    use std::os::unix::fs::MetadataExt;
    let uid = unsafe { libc::geteuid() };
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| format!("/run/user/{uid}").into());
    let metadata = std::fs::symlink_metadata(&base).map_err(|_| "runtime_unavailable")?;
    if !base.is_absolute()
        || !metadata.is_dir()
        || metadata.uid() != uid
        || metadata.mode() & 0o077 != 0
    {
        return Err("runtime_permissions");
    }
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn assuan_decodes_percent_but_not_plus() {
        assert_eq!(
            &*decode_data(b"fixture%25+%20value").unwrap(),
            b"fixture%+ value"
        );
        assert!(decode_data(b"fixture%XY").is_err());
    }
    #[test]
    fn session_secret_rejects_empty_and_multiline() {
        assert!(SessionSecret::new(b"").is_err());
        assert!(SessionSecret::new(b"fixture\nvalue").is_err());
        let secret = SessionSecret::new(b"synthetic fixture").unwrap();
        assert_eq!(secret.expose(), b"synthetic fixture");
        assert!(secrets_equal(&secret, &secret));
        assert!(!secrets_equal(
            &secret,
            &SessionSecret::new(b"synthetic fixturE").unwrap()
        ));
        assert!(!secrets_equal(
            &secret,
            &SessionSecret::new(b"shorter").unwrap()
        ));
    }
}
