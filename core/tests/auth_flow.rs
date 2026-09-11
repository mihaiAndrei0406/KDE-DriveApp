//! Offline end-to-end authentication using only a temporary encrypted fixture.
use hetzner_drive_core::{
    Paths,
    auth::{self, AuthOperation, EncryptedConfig},
    secrets::SessionSecret,
    user_home,
};
use std::{fs, os::unix::fs::PermissionsExt, process::Stdio};
use tokio::process::Command;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|s| s == "--password-helper") {
        return auth::password_helper().await.map_err(Into::into);
    }
    if std::env::args().any(|s| s == "--fixture-password") {
        println!("synthetic-test-only-config-password");
        return Ok(());
    }
    let directory = tempfile::tempdir()?;
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))?;
    // This standalone test has no other threads reading the environment at this point.
    // Runtime sockets and the fixture are confined to this disposable directory.
    unsafe {
        std::env::set_var("XDG_RUNTIME_DIR", directory.path());
    }
    let config = directory.path().join("fixture.conf");
    let key_file = user_home().join(".ssh/fixture-storagebox");
    let paths = Paths {
        key_file: key_file.clone(),
        sftp_host: "u12345.your-storagebox.de".into(),
        sftp_user: "u12345".into(),
        ..Paths::default()
    };
    let fixture = format!(
        "[hetzner-crypt]\ntype = crypt\nremote = hetzner-raw:encrypted-data\npassword = fixture-not-used-for-data\npassword2 = fixture-not-used-for-data\n[hetzner-raw]\ntype = sftp\nhost = u12345.your-storagebox.de\nuser = u12345\nport = 23\nkey_use_agent = true\nkey_file = {}\nshell_type = unix\nhost_keys = ssh-ed25519 fixture-public-key\n",
        key_file.display()
    );
    fs::write(&config, fixture)?;
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600))?;
    let password = "synthetic-test-only-config-password";
    // This test-only helper supplies a synthetic value, never a user's password.
    let helper = format!(
        "\"{}\" --fixture-password",
        std::env::current_exe()?.display()
    );
    let status = Command::new("/usr/local/bin/rclone")
        .env_clear()
        .env("LC_ALL", "C")
        .arg("--password-command")
        .arg(helper)
        .args([
            "config",
            "encryption",
            "set",
            "--ask-password=false",
            "--config",
        ])
        .arg(&config)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await?;
    assert!(
        status.status.success(),
        "Fixture encryption failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let original = fs::read(&config)?;
    let snapshot = EncryptedConfig::capture(&config)?;
    let secret = SessionSecret::new(password.as_bytes())?;
    let redacted = auth::run(AuthOperation::ValidateConfig, &snapshot, &secret, &paths).await?;
    auth::validate_redacted(&redacted, &paths)?;
    assert!(!String::from_utf8_lossy(&redacted).contains(password));
    let wrong = SessionSecret::new(b"wrong-fixture-password")?;
    assert!(
        auth::run(AuthOperation::ValidateConfig, &snapshot, &wrong, &paths)
            .await
            .is_err()
    );
    assert_eq!(
        fs::read(&config)?,
        original,
        "Authentication changed the fixture config"
    );
    assert!(
        !fs::read_dir(directory.path())?.any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("hetzner-pass-")),
        "Password socket left behind"
    );
    println!(
        "PASS: real rclone encrypted fixture, authenticated one-shot helper, wrong password, unchanged config, socket cleanup"
    );
    Ok(())
}
