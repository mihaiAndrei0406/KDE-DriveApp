import argparse
import os
from pathlib import Path
import sys

from PySide6.QtCore import QDateTime, QEvent, QLockFile, QObject, QRunnable, Qt, QThreadPool, QTimer, Signal, Slot
from PySide6.QtGui import QAction, QCloseEvent, QIcon, QPalette
from PySide6.QtNetwork import QLocalServer, QLocalSocket
from PySide6.QtWidgets import (
    QAbstractButton, QApplication, QFileDialog, QFormLayout, QFrame, QGridLayout, QHBoxLayout, QHeaderView,
    QInputDialog, QLabel, QLayout, QMainWindow, QMenu, QMessageBox, QPlainTextEdit, QProgressBar,
    QPushButton, QScrollArea, QSizePolicy, QStyle, QSystemTrayIcon, QTabWidget, QTreeWidget,
    QTreeWidgetItem, QVBoxLayout, QWidget,
)

from .client import DriveClient
from .i18n import SUPPORTED_LANGUAGES, Translator, resolve_language
from .projects import ProjectError, ProjectMonitor

STATE_LABELS = {
    "Locked": "Deblocare necesara", "Ready": "Pregatit", "Mounting": "Se monteaza",
    "Mounted": "Montat", "Unmounting": "Se demonteaza", "Degraded": "Necesita atentie",
    "Error": "Verificare nereusita",
}
SSH_LABELS = {
    "operation_busy": "O alta operatie este in curs; verifica din nou dupa finalizare",
    "loaded": "Cheia Hetzner este incarcata", "key_missing": "Cheia Hetzner nu este incarcata",
    "agent_unavailable": "Agent SSH indisponibil", "public_key_unavailable": "Cheia publica nu poate fi verificata",
    "public_key_invalid": "Cheie publica nevalida", "agent_timeout": "Verificarea SSH a expirat",
    "agent_error": "Eroare la verificarea SSH",
}
CONFIG_LABELS = {
    "unlocked": "Deblocata pentru sesiune",
    "present_not_unlocked": "Prezenta; deblocare neverificata", "missing": "Lipseste",
    "unavailable": "Inaccesibila", "unsafe_path": "Cale nesigura",
    "unsafe_permissions": "Permisiuni sau proprietar necorespunzator",
}
ERROR_MESSAGES = {
    "configuration_locked": "Configuratia rclone este blocata.",
    "operation_busy": "O alta operatie este deja in curs.",
    "unlock_cancelled": "Deblocarea a fost anulata.",
    "unlock_timeout": "Timpul pentru introducerea parolei a expirat.",
    "unlock_failed": "Configuratia nu a putut fi deblocata. Verifica parola.",
    "config_policy_mismatch": "Configuratia difera de setupul Hetzner asteptat.",
    "config_override_forbidden": "Configuratia contine optiuni care suprascriu comenzile aplicatiei.",
    "password_not_requested": "rclone nu a confirmat utilizarea canalului securizat de deblocare.",
    "crypt_policy_mismatch": "Setarile de criptare nu corespund cerintelor.",
    "sftp_policy_mismatch": "Setarile SSH necesita verificare.",
    "transport_policy_missing": "Lipsesc hostul sau utilizatorul Storage Box din politica privata a serviciului.",
    "transport_policy_invalid": "Hostul, utilizatorul sau calea cheii SSH din politica privata nu sunt valide.",
    "host_pinning_missing": "Lipseste verificarea cheii serverului SSH.",
    "ssh_key_unavailable": "Cheia Hetzner nu este disponibila in agentul SSH.",
    "rclone_operation_failed": "Operatia rclone a esuat. Verifica autentificarea si conexiunea.",
    "rclone_timeout": "Conexiunea nu a raspuns la timp.",
    "secret_memory_lock_failed": "Memoria protejata pentru parola nu este disponibila.",
    "pinentry_unavailable": "Dialogul securizat pentru parola nu este disponibil.",
    "config_not_encrypted": "Fisierul nu este o configuratie rclone criptata compatibila.",
    "drive_not_mounted": "Drive-ul nu este montat.",
    "local_health_failed": "Verificarile locale au identificat o problema.",
    "storage_unavailable": "Informatiile despre spatiu nu sunt disponibile.",
    "storage_invalid": "Serviciul a returnat informatii despre spatiu nevalide.",
    "connection_response_invalid": "Conexiunea nu a confirmat directorul criptat asteptat.",
    "dolphin_unavailable": "Dolphin nu a putut fi pornit.",
    "rclone_start_failed": "rclone nu a putut fi pornit.",
    "mount_start_failed": "Procesul de mount nu a putut fi pornit.",
    "mount_timeout": "Mount-ul nu a devenit disponibil la timp.",
    "mount_process_exited": "Procesul de mount s-a oprit neasteptat.",
    "drive_already_mounted": "Drive-ul este deja montat de aplicatie.",
    "mount_path_conflict": "Alta resursa foloseste directorul de mount.",
    "remote_already_mounted_elsewhere": "Remote-ul este deja montat intr-un alt director.",
    "mount_directory_unsafe": "Directorul de mount nu are proprietarul, permisiunile sau calea asteptate.",
    "cache_directory_unsafe": "Directorul cache nu are proprietarul, permisiunile sau calea asteptate.",
    "mount_directory_not_empty": "Directorul de mount contine fisiere locale si nu va fi acoperit.",
    "mount_directory_unavailable": "Directorul de mount nu poate fi verificat.",
    "mount_state_unknown": "Starea mount-urilor Linux nu poate fi verificata sigur.",
    "mount_not_owned": "Drive-ul este montat extern; foloseste fluxul care l-a pornit pentru demontare.",
    "owned_mount_not_observed": "Procesul detinut nu mai corespunde mount-ului asteptat.",
    "uploads_pending": "Exista uploaduri in curs sau in asteptare. Demontarea a fost amanata.",
    "upload_errors": "Cache-ul VFS raporteaza fisiere cu erori. Drive-ul ramane montat.",
    "cache_out_of_space": "Cache-ul VFS nu mai are spatiu suficient. Drive-ul ramane montat.",
    "upload_state_unknown": "Starea uploadurilor nu poate fi confirmata. Drive-ul ramane montat.",
    "rc_unavailable": "Canalul privat de verificare al mount-ului nu raspunde.",
    "rc_authentication_failed": "Autentificarea canalului privat RC a esuat.",
    "rc_peer_failed": "Identitatea procesului de la celalalt capat al canalului RC nu corespunde mount-ului.",
    "rc_timeout": "Verificarea uploadurilor a expirat.",
    "rc_io_failed": "Comunicarea prin canalul privat RC a esuat.",
    "rc_response_invalid": "Canalul privat RC a returnat un raspuns nevalid.",
    "rc_response_failed": "Canalul privat RC a refuzat verificarea solicitata.",
    "rc_output_limit": "Raspunsul canalului privat RC a depasit limita acceptata.",
    "rc_socket_invalid": "Socketul privat RC nu are tipul asteptat.",
    "rc_socket_permissions": "Permisiunile socketului privat RC nu au putut fi securizate.",
    "rc_runtime_failed": "Directorul privat RC nu a putut fi creat in siguranta.",
    "unmount_busy_or_failed": "Drive-ul este ocupat sau demontarea a esuat.",
    "unmount_timeout": "Demontarea nu s-a terminat la timp.",
    "unmount_not_confirmed": "Linux nu a confirmat demontarea; drive-ul ramane tratat ca montat.",
    "restic_unavailable": "restic nu este instalat. Instaleaza pachetul Debian «restic».",
    "backup_dependency_unsafe": "Executabilul restic sau rclone nu a trecut verificarile de securitate.",
    "backup_repository_locked": "Repository-ul restic este blocat pentru aceasta sesiune.",
    "backup_unlock_cancelled": "Introducerea parolei repository-ului restic a fost anulata.",
    "backup_unlock_failed": "Repository-ul restic nu a putut fi deschis. Verifica parola separata restic.",
    "backup_password_mismatch": "Cele doua parole noi pentru repository-ul restic nu coincid.",
    "backup_repository_init_failed": "Repository-ul disposable nu a putut fi initializat.",
    "backup_confirmation_cancelled": "Operatia de backup a fost anulata.",
    "backup_confirmation_timeout": "Confirmarea operatiei de backup a expirat.",
    "backup_password_timeout": "Canalul securizat pentru parolele de backup nu a raspuns.",
    "backup_busy": "Un job restic este deja in curs.",
    "backup_failed": "Snapshotul restic nu a putut fi creat.",
    "backup_check_failed": "Snapshotul a fost creat, dar verificarea repository-ului a esuat.",
    "backup_summary_invalid": "restic nu a confirmat identificatorul snapshotului creat.",
    "backup_history_failed": "Istoricul snapshoturilor nu a putut fi citit din repository.",
    "backup_history_invalid": "Repository-ul a returnat un istoric de snapshoturi nevalid.",
    "backup_contents_failed": "Continutul snapshotului nu a putut fi citit din repository.",
    "backup_contents_invalid": "Repository-ul a returnat un arbore de fisiere nevalid.",
    "backup_entry_unavailable": "Fisierul sau directorul selectat nu mai este autorizat. Reincarca continutul snapshotului.",
    "project_registry_unavailable": "Registrul local al proiectelor nu este disponibil serviciului.",
    "project_registry_unsafe": "Registrul local al proiectelor are permisiuni nesigure.",
    "project_registry_invalid": "Registrul local al proiectelor nu respecta schema acceptata.",
    "project_registry_overlap": "Registrul contine directoare de proiect suprapuse.",
    "project_not_found": "Proiectul selectat nu mai exista.",
    "project_path_unavailable": "Directorul proiectului nu mai este disponibil.",
    "project_path_unsafe": "Calea proiectului s-a schimbat sau nu mai este sigura.",
    "backup_snapshot_unavailable": "Snapshotul ales nu mai este disponibil pentru acest proiect. Actualizeaza istoricul.",
    "restore_target_failed": "Directorul nou de restaurare nu a putut fi creat.",
    "restore_target_unsafe": "Directorul de restaurare nu are proprietarul sau permisiunile cerute.",
    "restore_summary_invalid": "restic nu a confirmat finalizarea completa a restaurarii.",
}
OPERATION_LABELS = {
    "unlock": "Deblocare configuratie", "lock_configuration": "Blocare configuratie",
    "check_connection": "Verificare conexiune", "storage_usage": "Verificare spatiu cloud",
    "health_check": "Diagnostic", "check_ssh": "Verificare SSH", "open_drive": "Deschidere in Dolphin",
    "mount_drive": "Montare drive", "unmount_drive": "Demontare drive",
}
MOUNT_LABELS = {
    "mounted": "Montat local; conexiune neverificata", "unmounted": "Demontat",
    "conflict": "Alt mount pe aceeasi cale", "unknown": "Stare necunoscuta",
}

LIGHT_THEME = {
    "window": "#F8F7FC", "surface": "#FFFFFF", "surface_alt": "#F2EEFA",
    "border": "#DDD6EB", "text": "#1B1427", "muted": "#6B6178",
    "disabled": "#9A91A5", "primary": "#6D28D9", "primary_hover": "#7C3AED",
    "primary_pressed": "#5B21B6", "primary_text": "#FFFFFF", "lavender": "#EDE9FE",
    "focus": "#A855F7", "success": "#0F766E", "warning": "#B45309",
    "error": "#BE123C", "info": "#0369A1",
}

DARK_THEME = {
    "window": "#100B18", "surface": "#191126", "surface_alt": "#241735",
    "border": "#3A294D", "text": "#F7F2FF", "muted": "#B9AEC8",
    "disabled": "#756A82", "primary": "#A855F7", "primary_hover": "#C084FC",
    "primary_pressed": "#9333EA", "primary_text": "#180B29", "lavender": "#C4B5FD",
    "focus": "#C4B5FD", "success": "#2DD4BF", "warning": "#FBBF24",
    "error": "#FB7185", "info": "#38BDF8",
}

STATE_TONES = {
    "Locked": "locked", "Ready": "ready", "Mounting": "working",
    "Mounted": "mounted", "Unmounting": "working", "Degraded": "warning",
    "Error": "error",
}

PROJECT_STATE_LABELS = {
    "verified": "Verificat",
    "changed": "Modificari detectate",
    "unprotected": "Backup initial necesar",
    "snoozed": "Amanat",
    "due": "Backup recomandat",
    "attention": "Scanare incompleta",
}

PROJECT_STATE_TONES = {
    "verified": "success",
    "changed": "info",
    "unprotected": "warning",
    "snoozed": "info",
    "due": "warning",
    "attention": "error",
}

BACKUP_PHASE_LABELS = {
    "unavailable": "Motor restic indisponibil",
    "locked": "Repository restic blocat",
    "ready": "Repository restic pregatit",
    "initializing": "Se initializeaza repository-ul disposable",
    "unlocking": "Se verifica parola restic",
    "history": "Se incarca istoricul snapshoturilor",
    "contents": "Se incarca continutul snapshotului",
    "backup": "Snapshot restic in curs",
    "checking": "Se verifica repository-ul",
    "backup_complete": "Snapshot creat si verificat structural",
    "restoring": "Restaurare intr-un director nou",
    "restore_complete": "Restaurare finalizata",
    "failed": "Operatia restic a esuat",
}


def theme_for_palette(palette):
    """Keep the purple identity while following KDE's light/dark preference."""
    return DARK_THEME if palette.color(QPalette.ColorRole.Window).lightness() < 128 else LIGHT_THEME


def build_stylesheet(theme):
    return f"""
    QMainWindow, QWidget#appRoot {{ background: {theme['window']}; color: {theme['text']}; }}
    QFrame#heroCard {{
        border: 1px solid #4C1D95; border-radius: 18px;
        background: qlineargradient(x1:0, y1:0, x2:1, y2:1,
                    stop:0 #24103F, stop:0.55 #3B1764, stop:1 #6D28D9);
    }}
    QLabel#heroEyebrow {{ color: #C4B5FD; font-size: 10px; font-weight: 700; }}
    QLabel#heroTitle {{ color: #FFFFFF; font-size: 24px; font-weight: 700; }}
    QLabel#heroSubtitle {{ color: #EDE9FE; }}
    QLabel#remoteChip {{
        color: #F7F2FF; background: rgba(16, 11, 24, 125);
        border: 1px solid rgba(196, 181, 253, 95); border-radius: 10px;
        padding: 5px 10px; font-family: monospace;
    }}
    QLabel#stateBadge {{ border-radius: 11px; padding: 5px 11px; font-weight: 700; }}
    QLabel#stateBadge[stateTone="locked"] {{ background: #EDE9FE; color: #3B1764; }}
    QLabel#stateBadge[stateTone="ready"] {{ background: #C4B5FD; color: #24103F; }}
    QLabel#stateBadge[stateTone="working"] {{ background: #A855F7; color: #180B29; }}
    QLabel#stateBadge[stateTone="mounted"] {{ background: #2DD4BF; color: #062E2A; }}
    QLabel#stateBadge[stateTone="warning"] {{ background: #FBBF24; color: #3A2100; }}
    QLabel#stateBadge[stateTone="error"] {{ background: #FB7185; color: #3C0912; }}
    QLabel#stateBadge[stateTone="offline"] {{ background: #FB7185; color: #3C0912; }}
    QLabel#errorBanner {{
        background: {theme['surface_alt']}; color: {theme['error']};
        border: 1px solid {theme['error']}; border-radius: 11px; padding: 9px 12px;
    }}
    QLabel#backupNotice {{
        background: {theme['surface_alt']}; color: {theme['muted']};
        border: 1px solid {theme['border']}; border-radius: 11px; padding: 9px 12px;
    }}
    QLabel#backupNotice[noticeTone="warning"] {{ color: {theme['warning']}; border-color: {theme['warning']}; }}
    QLabel#backupNotice[noticeTone="error"] {{ color: {theme['error']}; border-color: {theme['error']}; }}
    QLabel#backupNotice[noticeTone="success"] {{ color: {theme['success']}; border-color: {theme['success']}; }}
    QFrame#actionPanel, QFrame#panelCard {{
        background: {theme['surface']}; border: 1px solid {theme['border']}; border-radius: 14px;
    }}
    QLabel#sectionTitle {{ color: {theme['text']}; font-size: 14px; font-weight: 700; }}
    QLabel#sectionHint, QLabel#fieldLabel {{ color: {theme['muted']}; }}
    QLabel#metricValue {{ color: {theme['text']}; font-size: 15px; font-weight: 600; }}
    QLabel#metricValue[metricTone="success"] {{ color: {theme['success']}; }}
    QLabel#metricValue[metricTone="warning"] {{ color: {theme['warning']}; }}
    QLabel#metricValue[metricTone="error"] {{ color: {theme['error']}; }}
    QLabel#metricValue[metricTone="info"] {{ color: {theme['info']}; }}
    QPushButton {{
        min-height: 36px; padding: 0 13px; border-radius: 9px;
        border: 1px solid {theme['border']}; background: {theme['surface_alt']};
        color: {theme['text']}; font-weight: 600;
    }}
    QPushButton:hover {{ border-color: {theme['focus']}; background: {theme['lavender']}; color: #24103F; }}
    QPushButton:pressed {{ background: {theme['border']}; }}
    QPushButton:focus {{ border: 2px solid {theme['focus']}; }}
    QPushButton:disabled {{ color: {theme['disabled']}; background: {theme['surface_alt']}; border-color: {theme['border']}; }}
    QPushButton[buttonRole="primary"] {{
        background: {theme['primary']}; border-color: {theme['primary']}; color: {theme['primary_text']};
    }}
    QPushButton[buttonRole="primary"]:hover {{ background: {theme['primary_hover']}; border-color: {theme['primary_hover']}; color: {theme['primary_text']}; }}
    QPushButton[buttonRole="primary"]:pressed {{ background: {theme['primary_pressed']}; }}
    QPushButton[buttonRole="danger"] {{ color: {theme['warning']}; border-color: {theme['warning']}; background: transparent; }}
    QPushButton[buttonRole="quiet"] {{ background: transparent; }}
    QPushButton[buttonRole="primary"]:disabled, QPushButton[buttonRole="danger"]:disabled {{
        color: {theme['disabled']}; background: {theme['surface_alt']}; border-color: {theme['border']};
    }}
    QProgressBar {{
        min-height: 8px; max-height: 8px; border: 0; border-radius: 4px;
        background: {theme['surface_alt']}; text-align: center;
    }}
    QProgressBar::chunk {{ border-radius: 4px; background: {theme['primary']}; }}
    QTabWidget::pane {{ border: 0; background: transparent; top: -1px; }}
    QTabBar::tab {{
        color: {theme['muted']}; background: transparent; border: 0;
        padding: 9px 16px; margin-right: 4px; font-weight: 600;
    }}
    QTabBar::tab:selected {{ color: {theme['primary']}; border-bottom: 2px solid {theme['primary']}; }}
    QTabBar::tab:hover {{ color: {theme['primary_hover']}; }}
    QScrollArea {{ background: transparent; border: 0; }}
    QPlainTextEdit {{
        color: {theme['text']}; background: {theme['surface']}; border: 1px solid {theme['border']};
        border-radius: 11px; padding: 9px; selection-background-color: {theme['primary']};
        font-family: monospace;
    }}
    QTreeWidget {{
        color: {theme['text']}; background: {theme['surface']}; border: 1px solid {theme['border']};
        border-radius: 11px; padding: 5px; outline: 0;
    }}
    QTreeWidget::item {{ min-height: 34px; padding: 3px 5px; border-radius: 6px; }}
    QTreeWidget::item:hover {{ background: {theme['surface_alt']}; }}
    QTreeWidget::item:selected {{ background: {theme['primary']}; color: {theme['primary_text']}; }}
    QHeaderView::section {{
        color: {theme['muted']}; background: {theme['surface']}; border: 0;
        border-bottom: 1px solid {theme['border']}; padding: 7px; font-weight: 600;
    }}
    QStatusBar {{ color: {theme['muted']}; background: {theme['window']}; border-top: 1px solid {theme['border']}; }}
    QToolTip {{ color: {theme['text']}; background: {theme['surface']}; border: 1px solid {theme['focus']}; padding: 5px; }}
    """


def size_text(value):
    size = float(value)
    for unit in ("B", "KiB", "MiB", "GiB", "TiB"):
        if size < 1024 or unit == "TiB":
            return f"{size:.1f} {unit}"
        size /= 1024


def elapsed_text(seconds, translate=lambda text: text):
    seconds = max(0, int(seconds))
    if seconds < 60:
        return translate("mai putin de un minut")
    minutes = seconds // 60
    if minutes < 60:
        return f"{minutes} min"
    hours, minutes = divmod(minutes, 60)
    if hours < 24:
        return f"{hours}h {minutes:02d}m"
    days, hours = divmod(hours, 24)
    return translate("{days}z {hours}h").format(days=days, hours=hours)


class ProjectScanSignals(QObject):
    finished = Signal(object)
    failed = Signal(str)


class ProjectScanTask(QRunnable):
    def __init__(self, monitor):
        super().__init__()
        self.monitor = monitor
        self.signals = ProjectScanSignals()

    @Slot()
    def run(self):
        try:
            self.signals.finished.emit(self.monitor.refresh())
        except ProjectError as error:
            self.signals.failed.emit(str(error))
        except Exception:
            self.signals.failed.emit("Monitorizarea proiectelor a intampinat o eroare locala.")


class SingleInstance(QObject):
    """Per-user local activation channel for the GUI/tray process."""

    activate_requested = Signal()

    def __init__(self, name=None, lock_path=None, parent=None):
        super().__init__(parent)
        self.name = name or f"hetzner-drive-gui-{os.getuid()}"
        runtime = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
        self.lock = QLockFile(str(lock_path or runtime / f"{self.name}.lock"))
        self.server = QLocalServer(self)
        self.server.setSocketOptions(QLocalServer.SocketOption.UserAccessOption)
        self.server.newConnection.connect(self._accept_activation)

    def acquire(self):
        if not self.lock.tryLock(0):
            self._notify_existing()
            return False
        QLocalServer.removeServer(self.name)
        if self.server.listen(self.name):
            return True
        self.lock.unlock()
        return False

    def _notify_existing(self):
        socket = QLocalSocket(self)
        socket.connectToServer(self.name)
        connected = socket.waitForConnected(250)
        if connected:
            socket.write(b"activate")
            socket.waitForBytesWritten(250)
            socket.disconnectFromServer()
        socket.deleteLater()
        return connected

    def _accept_activation(self):
        while self.server.hasPendingConnections():
            socket = self.server.nextPendingConnection()
            if socket is not None:
                socket.disconnectFromServer()
                socket.deleteLater()
        self.activate_requested.emit()


class DriveWindow(QMainWindow):
    def __init__(self, client=None, project_monitor=None, language="ro"):
        super().__init__()
        self.translator = Translator(language)
        self._ = self.translator.text
        self.client = client or DriveClient(self)
        self.setWindowTitle("Hetzner Drive")
        self.resize(800, 610)
        self.setMinimumSize(540, 480)
        self.previous_state = None
        self.latest_status = None
        self.client_busy = False
        self.service_busy = False
        self.owned_mount = False
        self.operation_details = ""
        self.tray = None
        self.project_monitor = project_monitor or ProjectMonitor()
        self.backup_scan_running = False
        self.project_statuses = []
        self.backup_scan_task = None
        self.pending_backup_notification = False
        self.backup_engine_status = None
        self.active_backup_digest = None
        self.pending_backup_verification = None
        self.handled_backup_snapshot = ""
        self.snapshot_history_project_id = ""
        self.snapshot_history_loading = False
        self.snapshot_history_refresh_pending = False
        self.history_refresh_snapshot = ""
        self.snapshot_entries_key = ("", "")
        self.snapshot_entries_loading = False
        self.snapshot_entries_refresh_pending = False
        self.thread_pool = QThreadPool.globalInstance()
        self.setWindowIcon(self.icon("drive-harddisk", QStyle.StandardPixmap.SP_DriveHDIcon))

        body = QWidget()
        body.setObjectName("appRoot")
        self.setCentralWidget(body)
        layout = QVBoxLayout(body)
        layout.setContentsMargins(22, 18, 22, 12)
        layout.setSpacing(13)

        hero = QFrame()
        hero.setObjectName("heroCard")
        hero_layout = QHBoxLayout(hero)
        hero_layout.setContentsMargins(20, 16, 18, 16)
        hero_layout.setSpacing(14)
        hero_copy = QVBoxLayout()
        hero_copy.setSpacing(2)
        eyebrow = QLabel("SECURE CLOUD STORAGE")
        eyebrow.setObjectName("heroEyebrow")
        title = QLabel("Hetzner Drive")
        title.setObjectName("heroTitle")
        subtitle = QLabel("Stocare criptata local, controlata din KDE")
        subtitle.setObjectName("heroSubtitle")
        subtitle.setWordWrap(True)
        hero_copy.addWidget(eyebrow)
        hero_copy.addWidget(title)
        hero_copy.addWidget(subtitle)
        hero_layout.addLayout(hero_copy, 1)
        hero_status = QVBoxLayout()
        hero_status.setSpacing(7)
        hero_status.setAlignment(Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter)
        remote = QLabel("hetzner-crypt:")
        remote.setObjectName("remoteChip")
        remote.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        remote.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.state_label = QLabel("Verificare in curs")
        self.state_label.setObjectName("stateBadge")
        self.state_label.setProperty("stateTone", "working")
        self.state_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.state_label.setWordWrap(True)
        hero_status.addWidget(remote)
        hero_status.addWidget(self.state_label)
        hero_layout.addLayout(hero_status)
        layout.addWidget(hero)

        self.error_label = QLabel()
        self.error_label.setObjectName("errorBanner")
        self.error_label.setTextFormat(Qt.TextFormat.PlainText)
        self.error_label.setWordWrap(True)
        self.error_label.hide()
        layout.addWidget(self.error_label)

        action_panel = QFrame()
        action_panel.setObjectName("actionPanel")
        action_panel_layout = QVBoxLayout(action_panel)
        action_panel_layout.setContentsMargins(13, 10, 13, 12)
        action_panel_layout.setSpacing(9)
        actions = QHBoxLayout()
        actions.setSpacing(8)
        action_title = QLabel("Actiuni rapide")
        action_title.setObjectName("sectionTitle")
        actions.addWidget(action_title)
        actions.addStretch()
        self.refresh_button = QPushButton(self.icon("view-refresh", QStyle.StandardPixmap.SP_BrowserReload), "Actualizeaza")
        self.refresh_button.setProperty("buttonRole", "quiet")
        self.refresh_button.clicked.connect(self.client.refresh)
        self.ssh_button = QPushButton(self.icon("dialog-password", QStyle.StandardPixmap.SP_FileDialogDetailedView), "Verifica SSH")
        self.ssh_button.setProperty("buttonRole", "quiet")
        self.ssh_button.clicked.connect(self.client.check_ssh)
        actions.addWidget(self.refresh_button)
        actions.addWidget(self.ssh_button)
        action_panel_layout.addLayout(actions)

        operations = QGridLayout()
        operations.setHorizontalSpacing(8)
        operations.setVerticalSpacing(8)
        self.operation_buttons = {}
        for index, (key, label, icon, fallback, callback) in enumerate((
            ("unlock", "Deblocheaza", "object-unlocked", QStyle.StandardPixmap.SP_DialogApplyButton, self.client.unlock),
            ("lock", "Blocheaza", "object-locked", QStyle.StandardPixmap.SP_DialogCloseButton, self.client.lock),
            ("mount", "Monteaza", "media-mount", QStyle.StandardPixmap.SP_DriveNetIcon, self.client.mount),
            ("unmount", "Demonteaza", "media-eject", QStyle.StandardPixmap.SP_DialogCancelButton, self.request_unmount),
            ("open", "Deschide", "folder-open", QStyle.StandardPixmap.SP_DirOpenIcon, self.client.open_drive),
            ("connection", "Conexiune", "network-connect", QStyle.StandardPixmap.SP_DriveNetIcon, self.client.check_connection),
            ("storage", "Spatiu cloud", "drive-harddisk", QStyle.StandardPixmap.SP_DriveHDIcon, self.client.get_storage),
            ("health", "Diagnostic", "dialog-information", QStyle.StandardPixmap.SP_MessageBoxInformation, self.client.health_check),
        )):
            button = QPushButton(self.icon(icon, fallback), label)
            button.setProperty("buttonRole", "danger" if key == "unmount" else "secondary")
            button.clicked.connect(callback)
            button.setEnabled(False)
            if key == "lock":
                button.setToolTip("Blocheaza configuratia in controller; un drive deja montat ramane accesibil")
            operations.addWidget(button, index // 4, index % 4)
            operations.setColumnStretch(index % 4, 1)
            self.operation_buttons[key] = button
        action_panel_layout.addLayout(operations)
        layout.addWidget(action_panel)

        self.tabs = QTabWidget()
        self.tabs.setDocumentMode(True)
        layout.addWidget(self.tabs, 1)
        overview = QWidget()
        overview_layout = QVBoxLayout(overview)
        overview_layout.setContentsMargins(0, 12, 0, 8)
        overview_layout.setSpacing(11)
        self.values = {}

        def value_widget(key):
            widget = QLabel("In asteptare")
            widget.setObjectName("metricValue")
            widget.setTextFormat(Qt.TextFormat.PlainText)
            widget.setWordWrap(True)
            widget.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
            widget.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Minimum)
            self.values[key] = widget
            return widget

        def card(title_text, hint_text=""):
            frame = QFrame()
            frame.setObjectName("panelCard")
            card_layout = QVBoxLayout(frame)
            card_layout.setContentsMargins(15, 13, 15, 14)
            card_layout.setSpacing(7)
            heading = QLabel(title_text)
            heading.setObjectName("sectionTitle")
            card_layout.addWidget(heading)
            if hint_text:
                hint = QLabel(hint_text)
                hint.setObjectName("sectionHint")
                hint.setWordWrap(True)
                card_layout.addWidget(hint)
            return frame, card_layout

        summary = QGridLayout()
        summary.setHorizontalSpacing(11)
        summary.setVerticalSpacing(11)
        storage_card, storage_layout = card("Spatiu cloud", "Capacitatea remote-ului criptat")
        storage_layout.addWidget(value_widget("storage"))
        self.storage_progress = QProgressBar()
        self.storage_progress.setRange(0, 1000)
        self.storage_progress.setValue(0)
        self.storage_progress.setTextVisible(False)
        self.storage_progress.setAccessibleName("Procent spatiu cloud utilizat")
        storage_layout.addWidget(self.storage_progress)
        summary.addWidget(storage_card, 0, 0)

        transfer_card, transfer_layout = card("Transferuri", "Activitate observata prin VFS")
        transfer_layout.addWidget(value_widget("transfers"))
        cache_caption = QLabel("Cache pe disc")
        cache_caption.setObjectName("fieldLabel")
        transfer_layout.addWidget(cache_caption)
        transfer_layout.addWidget(value_widget("cache"))
        summary.addWidget(transfer_card, 0, 1)
        summary.setColumnStretch(0, 1)
        summary.setColumnStretch(1, 1)
        overview_layout.addLayout(summary)

        system_card, system_layout = card("Sistem si securitate")
        system_form = QFormLayout()
        system_form.setVerticalSpacing(10)
        system_form.setSizeConstraint(QLayout.SizeConstraint.SetMinimumSize)
        system_form.setFieldGrowthPolicy(QFormLayout.FieldGrowthPolicy.AllNonFixedFieldsGrow)
        system_form.setRowWrapPolicy(QFormLayout.RowWrapPolicy.WrapLongRows)
        for key, label in (
            ("mount", "Drive"), ("ssh", "SSH"), ("config", "Configuratie rclone"),
            ("version", "Versiune"), ("mount_path", "Director local"),
        ):
            caption = QLabel(label)
            caption.setObjectName("fieldLabel")
            system_form.addRow(caption, value_widget(key))
        system_layout.addLayout(system_form)
        overview_layout.addWidget(system_card)

        activity_card, activity_layout = card("Activitate recenta")
        activity_form = QFormLayout()
        activity_form.setVerticalSpacing(10)
        activity_form.setRowWrapPolicy(QFormLayout.RowWrapPolicy.WrapLongRows)
        for key, label in (("last_operation", "Ultima operatie"), ("last_error", "Ultima eroare")):
            caption = QLabel(label)
            caption.setObjectName("fieldLabel")
            activity_form.addRow(caption, value_widget(key))
        activity_layout.addLayout(activity_form)
        overview_layout.addWidget(activity_card)
        overview_layout.addStretch()

        scroll = QScrollArea()
        scroll.setFrameShape(QFrame.Shape.NoFrame)
        scroll.setWidgetResizable(True)
        scroll.setWidget(overview)
        self.status_tab_index = self.tabs.addTab(scroll, "Stare")

        backup_page = QWidget()
        backup_layout = QVBoxLayout(backup_page)
        backup_layout.setContentsMargins(0, 12, 0, 8)
        backup_layout.setSpacing(10)
        backup_layout.setSizeConstraint(QLayout.SizeConstraint.SetMinimumSize)
        backup_header = QHBoxLayout()
        backup_heading = QVBoxLayout()
        backup_heading.setSpacing(2)
        backup_title = QLabel("Asistent backup proiecte")
        backup_title.setObjectName("sectionTitle")
        self.backup_summary = QLabel("Niciun proiect configurat")
        self.backup_summary.setObjectName("sectionHint")
        backup_heading.addWidget(backup_title)
        backup_heading.addWidget(self.backup_summary)
        backup_header.addLayout(backup_heading)
        backup_header.addStretch()
        self.add_project_button = QPushButton(
            self.icon("list-add", QStyle.StandardPixmap.SP_FileDialogNewFolder), "Adauga proiect"
        )
        self.add_project_button.setProperty("buttonRole", "primary")
        self.add_project_button.clicked.connect(self.add_backup_project)
        backup_header.addWidget(self.add_project_button)
        backup_layout.addLayout(backup_header)

        self.backup_notice = QLabel(
            "Monitorizarea este locala. Uploadul ramane blocat pana la configurarea si validarea motorului restic."
        )
        self.backup_notice.setObjectName("backupNotice")
        self.backup_notice.setProperty("noticeTone", "warning")
        self.backup_notice.setWordWrap(True)
        backup_layout.addWidget(self.backup_notice)

        self.backup_progress_label = QLabel("Motorul restic nu a fost verificat")
        self.backup_progress_label.setObjectName("sectionHint")
        self.backup_progress_label.setTextInteractionFlags(Qt.TextInteractionFlag.TextSelectableByMouse)
        self.backup_progress_label.setWordWrap(True)
        backup_layout.addWidget(self.backup_progress_label)
        self.backup_progress = QProgressBar()
        self.backup_progress.setRange(0, 1000)
        self.backup_progress.setValue(0)
        self.backup_progress.setTextVisible(False)
        self.backup_progress.setAccessibleName("Progres backup sau restaurare")
        self.backup_progress.hide()
        backup_layout.addWidget(self.backup_progress)

        self.backup_empty = QLabel(
            "Adauga explicit un director de lucru. Aplicatia va urmari numai nume si metadate, "
            "va ignora directoarele generate si te va anunta dupa intervalul configurat."
        )
        self.backup_empty.setObjectName("sectionHint")
        self.backup_empty.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.backup_empty.setWordWrap(True)
        self.backup_empty.setMinimumHeight(110)
        backup_layout.addWidget(self.backup_empty, 1)

        self.project_tree = QTreeWidget()
        self.project_tree.setColumnCount(4)
        self.project_tree.setHeaderLabels(("Proiect", "Stare", "Activitate", "Ultimul backup"))
        self.project_tree.setRootIsDecorated(False)
        self.project_tree.setAlternatingRowColors(False)
        self.project_tree.setSelectionMode(QTreeWidget.SelectionMode.SingleSelection)
        self.project_tree.setTextElideMode(Qt.TextElideMode.ElideMiddle)
        self.project_tree.setMinimumHeight(210)
        self.project_tree.header().setSectionResizeMode(0, QHeaderView.ResizeMode.Stretch)
        self.project_tree.header().setSectionResizeMode(1, QHeaderView.ResizeMode.ResizeToContents)
        self.project_tree.header().setSectionResizeMode(2, QHeaderView.ResizeMode.ResizeToContents)
        self.project_tree.header().setSectionResizeMode(3, QHeaderView.ResizeMode.ResizeToContents)
        self.project_tree.itemSelectionChanged.connect(self.project_selection_changed)
        self.project_tree.hide()
        backup_layout.addWidget(self.project_tree, 1)

        history_header = QHBoxLayout()
        history_heading = QLabel("Istoric snapshoturi")
        history_heading.setObjectName("sectionTitle")
        history_header.addWidget(history_heading)
        history_header.addStretch()
        self.refresh_history_button = QPushButton(
            self.icon("view-refresh", QStyle.StandardPixmap.SP_BrowserReload), "Actualizeaza istoricul"
        )
        self.refresh_history_button.setProperty("buttonRole", "quiet")
        self.refresh_history_button.clicked.connect(lambda: self.request_snapshot_history(force=True))
        history_header.addWidget(self.refresh_history_button)
        backup_layout.addLayout(history_header)
        self.snapshot_history_hint = QLabel(
            "Selecteaza un proiect si deblocheaza repository-ul restic pentru a vedea snapshoturile."
        )
        self.snapshot_history_hint.setObjectName("sectionHint")
        self.snapshot_history_hint.setWordWrap(True)
        backup_layout.addWidget(self.snapshot_history_hint)
        self.snapshot_tree = QTreeWidget()
        self.snapshot_tree.setColumnCount(4)
        self.snapshot_tree.setHeaderLabels(("Data", "Fisiere", "Dimensiune", "Snapshot"))
        self.snapshot_tree.setRootIsDecorated(False)
        self.snapshot_tree.setSelectionMode(QTreeWidget.SelectionMode.SingleSelection)
        self.snapshot_tree.setTextElideMode(Qt.TextElideMode.ElideMiddle)
        self.snapshot_tree.setMinimumHeight(150)
        self.snapshot_tree.header().setSectionResizeMode(0, QHeaderView.ResizeMode.Stretch)
        self.snapshot_tree.header().setSectionResizeMode(1, QHeaderView.ResizeMode.ResizeToContents)
        self.snapshot_tree.header().setSectionResizeMode(2, QHeaderView.ResizeMode.ResizeToContents)
        self.snapshot_tree.header().setSectionResizeMode(3, QHeaderView.ResizeMode.ResizeToContents)
        self.snapshot_tree.itemSelectionChanged.connect(self.snapshot_selection_changed)
        self.snapshot_tree.hide()
        backup_layout.addWidget(self.snapshot_tree, 1)

        contents_header = QHBoxLayout()
        contents_heading = QLabel("Continut snapshot")
        contents_heading.setObjectName("sectionTitle")
        contents_header.addWidget(contents_heading)
        contents_header.addStretch()
        self.refresh_contents_button = QPushButton(
            self.icon("view-refresh", QStyle.StandardPixmap.SP_BrowserReload), "Reincarca continutul"
        )
        self.refresh_contents_button.setProperty("buttonRole", "quiet")
        self.refresh_contents_button.clicked.connect(lambda: self.request_snapshot_entries(force=True))
        contents_header.addWidget(self.refresh_contents_button)
        backup_layout.addLayout(contents_header)
        self.snapshot_contents_hint = QLabel(
            "Selecteaza un snapshot pentru a incarca fisierele si directoarele restaurabile."
        )
        self.snapshot_contents_hint.setObjectName("sectionHint")
        self.snapshot_contents_hint.setWordWrap(True)
        backup_layout.addWidget(self.snapshot_contents_hint)
        self.snapshot_contents_tree = QTreeWidget()
        self.snapshot_contents_tree.setColumnCount(3)
        self.snapshot_contents_tree.setHeaderLabels(("Nume", "Tip", "Dimensiune"))
        self.snapshot_contents_tree.setRootIsDecorated(True)
        self.snapshot_contents_tree.setSelectionMode(QTreeWidget.SelectionMode.SingleSelection)
        self.snapshot_contents_tree.setTextElideMode(Qt.TextElideMode.ElideMiddle)
        self.snapshot_contents_tree.setMinimumHeight(160)
        self.snapshot_contents_tree.header().setSectionResizeMode(0, QHeaderView.ResizeMode.Stretch)
        self.snapshot_contents_tree.header().setSectionResizeMode(1, QHeaderView.ResizeMode.ResizeToContents)
        self.snapshot_contents_tree.header().setSectionResizeMode(2, QHeaderView.ResizeMode.ResizeToContents)
        self.snapshot_contents_tree.itemSelectionChanged.connect(self.update_backup_actions)
        self.snapshot_contents_tree.hide()
        backup_layout.addWidget(self.snapshot_contents_tree, 1)

        self.backup_actions_layout = QGridLayout()
        self.backup_actions_layout.setHorizontalSpacing(8)
        self.backup_actions_layout.setVerticalSpacing(8)
        self.scan_projects_button = QPushButton(
            self.icon("view-refresh", QStyle.StandardPixmap.SP_BrowserReload), "Scaneaza acum"
        )
        self.scan_projects_button.setProperty("buttonRole", "quiet")
        self.scan_projects_button.clicked.connect(self.refresh_backup_projects)
        self.snooze_project_button = QPushButton(
            self.icon("appointment-soon", QStyle.StandardPixmap.SP_BrowserStop), "Amana o ora"
        )
        self.snooze_project_button.clicked.connect(self.snooze_backup_project)
        self.remove_project_button = QPushButton(
            self.icon("list-remove", QStyle.StandardPixmap.SP_TrashIcon), "Elimina"
        )
        self.remove_project_button.setProperty("buttonRole", "danger")
        self.remove_project_button.clicked.connect(self.remove_backup_project)
        self.backup_now_button = QPushButton(
            self.icon("cloud-upload", QStyle.StandardPixmap.SP_ArrowUp), "Backup acum"
        )
        self.backup_now_button.setProperty("buttonRole", "primary")
        self.backup_now_button.clicked.connect(self.start_selected_backup)
        self.restore_backup_button = QPushButton(
            self.icon("document-revert", QStyle.StandardPixmap.SP_DialogResetButton), "Restaureaza tot"
        )
        self.restore_backup_button.clicked.connect(self.restore_selected_backup)
        self.restore_selection_button = QPushButton(
            self.icon("edit-select", QStyle.StandardPixmap.SP_FileIcon), "Restaureaza selectia"
        )
        self.restore_selection_button.clicked.connect(self.restore_selected_entry)
        self.initialize_backup_button = QPushButton(
            self.icon("document-new", QStyle.StandardPixmap.SP_FileDialogNewFolder), "Initializeaza restic"
        )
        self.initialize_backup_button.clicked.connect(self.initialize_backup_repository)
        self.unlock_backup_button = QPushButton(
            self.icon("object-unlocked", QStyle.StandardPixmap.SP_DialogApplyButton), "Deblocheaza restic"
        )
        self.unlock_backup_button.clicked.connect(self.client.unlock_backup_repository)
        self.lock_backup_button = QPushButton(
            self.icon("object-locked", QStyle.StandardPixmap.SP_DialogCloseButton), "Blocheaza restic"
        )
        self.lock_backup_button.clicked.connect(self.client.lock_backup_repository)
        self.backup_action_buttons = (
            self.scan_projects_button,
            self.snooze_project_button,
            self.remove_project_button,
            self.initialize_backup_button,
            self.unlock_backup_button,
            self.lock_backup_button,
            self.backup_now_button,
            self.restore_backup_button,
            self.restore_selection_button,
        )
        self.backup_actions_compact = None
        self.layout_backup_actions(self.width() < 700)
        backup_layout.addLayout(self.backup_actions_layout)
        backup_scroll = QScrollArea()
        backup_scroll.setFrameShape(QFrame.Shape.NoFrame)
        backup_scroll.setWidgetResizable(True)
        backup_scroll.setWidget(backup_page)
        self.backup_tab_index = self.tabs.addTab(backup_scroll, "Backupuri")

        logs_page = QWidget()
        log_layout = QVBoxLayout(logs_page)
        log_layout.setContentsMargins(0, 12, 0, 8)
        log_header = QHBoxLayout()
        log_title = QLabel("Evenimente securizate")
        log_title.setObjectName("sectionTitle")
        log_header.addWidget(log_title)
        log_header.addStretch()
        self.log_button = QPushButton(self.icon("view-refresh", QStyle.StandardPixmap.SP_BrowserReload), "Actualizeaza evenimentele")
        self.log_button.setProperty("buttonRole", "quiet")
        self.log_button.clicked.connect(self.client.get_logs)
        log_header.addWidget(self.log_button)
        log_layout.addLayout(log_header)
        self.log_view = QPlainTextEdit()
        self.log_view.setReadOnly(True)
        self.log_view.setMaximumBlockCount(100)
        log_layout.addWidget(self.log_view)
        self.logs_tab_index = self.tabs.addTab(logs_page, "Evenimente")

        details_page = QWidget()
        details_layout = QVBoxLayout(details_page)
        details_layout.setContentsMargins(0, 12, 0, 8)
        details_title = QLabel("Diagnostic tehnic")
        details_title.setObjectName("sectionTitle")
        details_layout.addWidget(details_title)
        self.details = QPlainTextEdit()
        self.details.setReadOnly(True)
        self.details.setMaximumBlockCount(30)
        details_layout.addWidget(self.details)
        self.details_tab_index = self.tabs.addTab(details_page, "Diagnostic")
        self.tabs.currentChanged.connect(self.tab_changed)

        self.statusBar().showMessage(self._("Conectare la serviciu..."))
        self.mode_label = QLabel()
        self.mode_label.setObjectName("sectionHint")
        self.statusBar().addPermanentWidget(self.mode_label)
        self.client.status_received.connect(self.apply_status)
        self.client.logs_received.connect(self.apply_logs)
        self.client.ssh_received.connect(self.apply_ssh)
        self.client.failed.connect(self.show_error)
        self.client.busy_changed.connect(self.set_busy)
        self.client.storage_received.connect(self.apply_storage)
        self.client.operation_received.connect(self.apply_operation)
        self.client.operation_status_received.connect(self.apply_operation_details)
        self.client.mount_activity_received.connect(self.apply_mount_activity)
        self.client.backup_status_received.connect(self.apply_backup_status)
        self.client.snapshot_history_received.connect(self.apply_snapshot_history)
        self.client.snapshot_entries_received.connect(self.apply_snapshot_entries)
        self.apply_theme()
        self.setup_tray()
        self.translate_static_ui()
        self.timer = QTimer(self)
        self.timer.setInterval(15000)
        self.timer.timeout.connect(self.client.refresh)
        self.timer.start()
        self.backup_timer = QTimer(self)
        self.backup_timer.setInterval(5 * 60 * 1000)
        self.backup_timer.timeout.connect(self.refresh_backup_projects)
        self.backup_timer.start()
        QTimer.singleShot(0, self.client.refresh)
        QTimer.singleShot(0, self.refresh_backup_projects)

    def icon(self, name, fallback):
        return QIcon.fromTheme(name, self.style().standardIcon(fallback))

    def translate_static_ui(self):
        """Translate labels created during setup; runtime messages translate at use."""
        for widget in self.findChildren(QLabel) + self.findChildren(QAbstractButton):
            if widget.text():
                widget.setText(self._(widget.text()))
            if widget.toolTip():
                widget.setToolTip(self._(widget.toolTip()))
            if widget.accessibleName():
                widget.setAccessibleName(self._(widget.accessibleName()))
        for tree in self.findChildren(QTreeWidget):
            header = tree.headerItem()
            for column in range(tree.columnCount()):
                header.setText(column, self._(header.text(column)))
        for index in range(self.tabs.count()):
            self.tabs.setTabText(index, self._(self.tabs.tabText(index)))
        for action in self.findChildren(QAction):
            action.setText(self._(action.text()))
            if action.toolTip():
                action.setToolTip(self._(action.toolTip()))

    def apply_theme(self):
        theme = theme_for_palette(QApplication.palette())
        if getattr(self, "theme", None) == theme:
            return
        self.theme = theme
        self.setStyleSheet(build_stylesheet(theme))

    @staticmethod
    def set_visual_property(widget, name, value):
        if widget.property(name) == value:
            return
        widget.setProperty(name, value)
        widget.style().unpolish(widget)
        widget.style().polish(widget)

    def changeEvent(self, event):
        super().changeEvent(event)
        if event.type() in (QEvent.Type.PaletteChange, QEvent.Type.ApplicationPaletteChange):
            QTimer.singleShot(0, self.apply_theme)

    def resizeEvent(self, event):
        super().resizeEvent(event)
        if hasattr(self, "backup_action_buttons"):
            self.layout_backup_actions(event.size().width() < 700)

    def layout_backup_actions(self, compact):
        if self.backup_actions_compact == compact:
            return
        self.backup_actions_compact = compact
        columns = 2 if compact else 4
        for index, button in enumerate(self.backup_action_buttons):
            self.backup_actions_layout.addWidget(button, index // columns, index % columns)
        for column in range(4):
            self.backup_actions_layout.setColumnStretch(column, 1 if column < columns else 0)

    def setup_tray(self):
        if not QSystemTrayIcon.isSystemTrayAvailable():
            return
        self.tray = QSystemTrayIcon(self.windowIcon(), self)
        self.tray.setToolTip("Hetzner Drive")
        menu = QMenu(self)
        for name, callback in (
            ("Deschide", self.reveal), ("Actualizeaza", self.client.refresh),
            ("Evenimente", self.show_logs),
        ):
            action = QAction(name, self)
            action.triggered.connect(callback)
            menu.addAction(action)
        menu.addSeparator()
        action = QAction("Iesire", self)
        action.triggered.connect(QApplication.instance().quit)
        menu.addAction(action)
        self.tray.setContextMenu(menu)
        self.tray.activated.connect(lambda reason: self.reveal() if reason == QSystemTrayIcon.ActivationReason.Trigger else None)
        self.tray.messageClicked.connect(self.backup_notification_clicked)
        self.tray.show()

    def reveal(self):
        self.showNormal()
        self.raise_()
        self.activateWindow()

    def show_logs(self):
        self.reveal()
        self.tabs.setCurrentIndex(self.logs_tab_index)

    def show_backups(self):
        self.reveal()
        self.tabs.setCurrentIndex(self.backup_tab_index)

    def backup_notification_clicked(self):
        if self.pending_backup_notification:
            self.pending_backup_notification = False
            self.show_backups()
        else:
            self.reveal()

    def tab_changed(self, index):
        if index == self.logs_tab_index:
            self.client.get_logs()

    def set_backup_notice(self, text, tone="warning"):
        self.backup_notice.setText(self._(text))
        self.set_visual_property(self.backup_notice, "noticeTone", tone)

    def selected_project_id(self):
        selected = self.project_tree.selectedItems()
        return selected[0].data(0, Qt.ItemDataRole.UserRole) if selected else None

    def selected_snapshot_id(self):
        selected = self.snapshot_tree.selectedItems()
        return selected[0].data(0, Qt.ItemDataRole.UserRole) if selected else None

    def selected_snapshot_entry_path(self):
        selected = self.snapshot_contents_tree.selectedItems()
        return selected[0].data(0, Qt.ItemDataRole.UserRole) if selected else None

    def clear_snapshot_entries(self, message):
        self.snapshot_contents_tree.clear()
        self.snapshot_contents_tree.hide()
        self.snapshot_contents_hint.setText(self._(message))
        self.snapshot_contents_hint.show()

    def clear_snapshot_history(self, message):
        self.snapshot_tree.clear()
        self.snapshot_tree.hide()
        self.snapshot_history_hint.setText(self._(message))
        self.snapshot_history_hint.show()
        self.snapshot_entries_key = ("", "")
        self.snapshot_entries_loading = False
        self.snapshot_entries_refresh_pending = False
        self.clear_snapshot_entries(
            "Selecteaza un snapshot pentru a incarca fisierele si directoarele restaurabile."
        )

    def project_selection_changed(self):
        self.snapshot_history_project_id = ""
        self.snapshot_history_refresh_pending = False
        self.clear_snapshot_history(
            "Deblocheaza repository-ul restic pentru a incarca snapshoturile proiectului selectat."
        )
        self.update_backup_actions()
        self.request_snapshot_history()

    def snapshot_selection_changed(self):
        self.snapshot_entries_key = ("", "")
        self.snapshot_entries_refresh_pending = False
        self.clear_snapshot_entries(
            "Se incarca fisierele si directoarele snapshotului selectat..."
            if self.selected_snapshot_id()
            else "Selecteaza un snapshot pentru a incarca fisierele si directoarele restaurabile."
        )
        self.update_backup_actions()
        self.request_snapshot_entries()

    def request_snapshot_entries(self, force=False):
        project_id = self.selected_project_id()
        snapshot_id = self.selected_snapshot_id()
        engine = self.backup_engine_status or {}
        config_unlocked = bool(self.latest_status and self.latest_status["config"] == "unlocked")
        if not project_id or not snapshot_id or not engine.get("repository_unlocked") or not config_unlocked:
            return
        if self.snapshot_entries_loading:
            self.snapshot_entries_refresh_pending |= force
            return
        if engine.get("busy") or self.client_busy:
            self.snapshot_entries_refresh_pending |= force
            return
        key = (project_id, snapshot_id)
        if not force and self.snapshot_entries_key == key:
            return
        self.snapshot_entries_loading = True
        self.snapshot_contents_hint.setText(
            self._("Se incarca fisierele si directoarele snapshotului selectat...")
        )
        self.snapshot_contents_hint.show()
        self.client.get_snapshot_entries(project_id, snapshot_id)
        self.update_backup_actions()

    def apply_snapshot_entries(self, project_id, snapshot_id, success, truncated, entries, reason):
        self.snapshot_entries_loading = False
        selected_key = (self.selected_project_id(), self.selected_snapshot_id())
        if (project_id, snapshot_id) != selected_key:
            self.snapshot_entries_key = ("", "")
            self.snapshot_entries_refresh_pending = False
            self.request_snapshot_entries(force=True)
            return
        self.snapshot_entries_key = selected_key if success else ("", "")
        self.snapshot_contents_tree.blockSignals(True)
        self.snapshot_contents_tree.clear()
        items = {}
        if success:
            for entry in entries:
                parts = entry["path"].split("/")
                parent_path = "/".join(parts[:-1])
                kind_label = self._("Director" if entry["kind"] == "dir" else "Fisier")
                item = QTreeWidgetItem([
                    parts[-1] if parent_path in items else entry["path"],
                    kind_label,
                    "" if entry["kind"] == "dir" else size_text(entry["size"]),
                ])
                item.setData(0, Qt.ItemDataRole.UserRole, entry["path"])
                item.setToolTip(0, entry["path"])
                item.setIcon(
                    0,
                    self.icon(
                        "folder" if entry["kind"] == "dir" else "text-x-generic",
                        QStyle.StandardPixmap.SP_DirIcon
                        if entry["kind"] == "dir"
                        else QStyle.StandardPixmap.SP_FileIcon,
                    ),
                )
                parent = items.get(parent_path)
                if parent is None:
                    self.snapshot_contents_tree.addTopLevelItem(item)
                else:
                    parent.addChild(item)
                items[entry["path"]] = item
        self.snapshot_contents_tree.blockSignals(False)
        if not success:
            self.clear_snapshot_entries(
                ERROR_MESSAGES.get(reason, "Continutul snapshotului nu este disponibil.")
            )
        elif entries:
            template = (
                "Sunt afisate primele {count} intrari restaurabile; lista este trunchiata."
                if truncated
                else "{count} intrari restaurabile in snapshotul selectat."
            )
            self.snapshot_contents_hint.setText(self._(template).format(count=len(entries)))
            self.snapshot_contents_hint.show()
            self.snapshot_contents_tree.show()
            self.snapshot_contents_tree.expandToDepth(0)
        else:
            self.clear_snapshot_entries("Snapshotul nu contine fisiere sau directoare restaurabile.")
        refresh_pending = self.snapshot_entries_refresh_pending
        self.snapshot_entries_refresh_pending = False
        self.update_backup_actions()
        self.client.refresh()
        if refresh_pending:
            QTimer.singleShot(0, lambda: self.request_snapshot_entries(force=True))

    def request_snapshot_history(self, force=False):
        project_id = self.selected_project_id()
        engine = self.backup_engine_status or {}
        config_unlocked = bool(self.latest_status and self.latest_status["config"] == "unlocked")
        if not project_id or not engine.get("repository_unlocked") or not config_unlocked:
            return
        if self.snapshot_history_loading:
            self.snapshot_history_refresh_pending |= force
            return
        if engine.get("busy") or self.client_busy:
            self.snapshot_history_refresh_pending |= force
            return
        if not force and self.snapshot_history_project_id == project_id:
            return
        self.snapshot_history_loading = True
        self.snapshot_history_hint.setText(self._("Se incarca istoricul snapshoturilor..."))
        self.snapshot_history_hint.show()
        self.client.get_project_snapshots(project_id)
        self.update_backup_actions()

    def apply_snapshot_history(self, project_id, success, truncated, entries, reason):
        self.snapshot_history_loading = False
        selected_project = self.selected_project_id()
        if project_id != selected_project:
            self.snapshot_history_project_id = ""
            self.snapshot_history_refresh_pending = False
            self.request_snapshot_history(force=True)
            return
        self.snapshot_history_project_id = project_id if success else ""
        self.snapshot_tree.blockSignals(True)
        self.snapshot_tree.clear()
        if success:
            for entry in entries:
                timestamp = QDateTime.fromString(entry["created_at"], Qt.DateFormat.ISODateWithMs)
                if not timestamp.isValid():
                    timestamp = QDateTime.fromString(entry["created_at"], Qt.DateFormat.ISODate)
                created = (
                    timestamp.toLocalTime().toString("dd.MM.yyyy HH:mm")
                    if timestamp.isValid()
                    else entry["created_at"]
                )
                item = QTreeWidgetItem([
                    created,
                    str(entry["files"]),
                    size_text(entry["bytes"]),
                    entry["id"][:12],
                ])
                item.setData(0, Qt.ItemDataRole.UserRole, entry["id"])
                item.setToolTip(3, entry["id"])
                self.snapshot_tree.addTopLevelItem(item)
        self.snapshot_tree.blockSignals(False)
        if not success:
            self.clear_snapshot_history(
                ERROR_MESSAGES.get(reason, "Istoricul snapshoturilor nu este disponibil.")
            )
        elif entries:
            template = (
                "Sunt afisate cele mai recente {count} snapshoturi."
                if truncated
                else "{count} snapshoturi disponibile pentru proiectul selectat."
            )
            self.snapshot_history_hint.setText(self._(template).format(count=len(entries)))
            self.snapshot_history_hint.show()
            self.snapshot_tree.show()
            self.snapshot_tree.setCurrentItem(self.snapshot_tree.topLevelItem(0))
        else:
            self.clear_snapshot_history("Nu exista snapshoturi pentru proiectul selectat.")
        refresh_pending = self.snapshot_history_refresh_pending
        self.snapshot_history_refresh_pending = False
        self.update_backup_actions()
        self.client.refresh()
        if refresh_pending:
            QTimer.singleShot(0, lambda: self.request_snapshot_history(force=True))

    def update_backup_actions(self):
        selected = self.selected_project_id() is not None
        selected_status = next(
            (status for status in self.project_statuses if status.id == self.selected_project_id()), None
        )
        engine = self.backup_engine_status or {}
        engine_available = bool(engine.get("available"))
        engine_unlocked = bool(engine.get("repository_unlocked"))
        engine_busy = bool(engine.get("busy"))
        config_unlocked = bool(self.latest_status and self.latest_status["config"] == "unlocked")
        controls_available = not self.backup_scan_running and not engine_busy and not self.client_busy
        self.snooze_project_button.setEnabled(selected and not self.backup_scan_running)
        self.remove_project_button.setEnabled(selected and not self.backup_scan_running)
        self.add_project_button.setEnabled(not self.backup_scan_running)
        self.scan_projects_button.setEnabled(not self.backup_scan_running)
        self.initialize_backup_button.setEnabled(
            engine_available and config_unlocked and not engine_unlocked and controls_available
        )
        self.unlock_backup_button.setEnabled(
            engine_available and config_unlocked and not engine_unlocked and controls_available
        )
        self.lock_backup_button.setEnabled(engine_unlocked and controls_available)
        self.refresh_history_button.setEnabled(
            selected and engine_unlocked and config_unlocked and controls_available
        )
        selected_snapshot = self.selected_snapshot_id()
        self.refresh_contents_button.setEnabled(
            selected
            and selected_snapshot is not None
            and engine_unlocked
            and config_unlocked
            and controls_available
            and not self.snapshot_entries_loading
        )
        safe_project = bool(selected_status and selected_status.state != "attention")
        self.backup_now_button.setEnabled(
            safe_project and engine_unlocked and config_unlocked and controls_available
        )
        self.restore_backup_button.setEnabled(
            selected
            and selected_snapshot is not None
            and engine_unlocked
            and config_unlocked
            and controls_available
        )
        self.restore_selection_button.setEnabled(
            selected
            and selected_snapshot is not None
            and self.selected_snapshot_entry_path() is not None
            and self.snapshot_entries_key == (self.selected_project_id(), selected_snapshot)
            and engine_unlocked
            and config_unlocked
            and controls_available
        )

    def refresh_backup_projects(self):
        if self.backup_scan_running:
            return
        try:
            projects = self.project_monitor.projects()
        except ProjectError as error:
            self.backup_scan_failed(str(error))
            return
        if not projects:
            self.apply_project_statuses([])
            return
        self.backup_scan_running = True
        self.backup_summary.setText(self._("Scanare metadate in curs..."))
        self.update_backup_actions()
        task = ProjectScanTask(self.project_monitor)
        task.signals.finished.connect(self.backup_scan_finished)
        task.signals.failed.connect(self.backup_scan_failed)
        self.backup_scan_task = task
        self.thread_pool.start(task)

    def backup_scan_finished(self, statuses):
        self.backup_scan_running = False
        self.backup_scan_task = None
        self.apply_project_statuses(statuses)
        if self.pending_backup_verification:
            project_id, expected_digest = self.pending_backup_verification
            self.pending_backup_verification = None
            try:
                self.project_monitor.mark_verified(project_id, expected_digest=expected_digest)
            except ProjectError as error:
                self.set_backup_notice(str(error), "warning")
            else:
                self.refresh_backup_projects()
                return
        try:
            candidates = self.project_monitor.notification_candidates(statuses)
        except ProjectError as error:
            self.set_backup_notice(str(error), "error")
            return
        if candidates and self.tray and QSystemTrayIcon.supportsMessages():
            if len(candidates) == 1:
                project = candidates[0]
                title = self._("Backup recomandat: {name}").format(name=project.name)
                age = elapsed_text(QDateTime.currentSecsSinceEpoch() - project.dirty_since, self._)
                message = self._(
                    "{count} fisiere monitorizate; exista modificari de {age}. "
                    "Deschide pagina Backupuri pentru detalii."
                ).format(count=project.file_count, age=age)
            else:
                title = self._("Backup recomandat")
                message = self._("{count} proiecte au modificari care necesita backup.").format(
                    count=len(candidates)
                )
            self.pending_backup_notification = True
            self.tray.showMessage(title, message, QSystemTrayIcon.MessageIcon.Warning, 15000)
            try:
                self.project_monitor.mark_notified([project.id for project in candidates])
            except ProjectError:
                pass
        self.update_backup_actions()

    def initialize_backup_repository(self):
        answer = QMessageBox.warning(
            self,
            self._("Repository restic disposable"),
            self._(
                "Aplicatia va crea un repository NOU in calea fixa de test:\n"
                "HetznerDrive-Backup-Disposable/restic-v1\n\n"
                "Vei introduce de doua ori o parola restic separata. Pastreaz-o in managerul tau de parole: "
                "fara ea, snapshoturile nu pot fi recuperate. Backendul este append-only si aceasta faza nu sterge date remote.\n\n"
                "Continui?"
            ),
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.Cancel,
            QMessageBox.StandardButton.Cancel,
        )
        if answer == QMessageBox.StandardButton.Yes:
            self.client.initialize_backup_repository()

    def start_selected_backup(self):
        project_id = self.selected_project_id()
        if not project_id:
            return
        try:
            digest = self.project_monitor.current_digest(project_id)
        except ProjectError as error:
            QMessageBox.warning(self, self._("Backup indisponibil"), self._(str(error)))
            return
        self.active_backup_digest = (project_id, digest)
        self.client.start_project_backup(project_id)

    def restore_selected_backup(self):
        project_id = self.selected_project_id()
        snapshot_id = self.selected_snapshot_id()
        if project_id and snapshot_id:
            self.client.start_backup_restore(project_id, snapshot_id)

    def restore_selected_entry(self):
        project_id = self.selected_project_id()
        snapshot_id = self.selected_snapshot_id()
        selected_path = self.selected_snapshot_entry_path()
        if not project_id or not snapshot_id or not selected_path:
            return
        answer = QMessageBox.warning(
            self,
            self._("Restaurare selectiva"),
            self._(
                "Restaurezi numai «{path}» din snapshot intr-un director local NOU. "
                "Originalul nu va fi suprascris, dar trebuie sa inspectezi rezultatul inainte de copiere.\n\nContinui?"
            ).format(path=selected_path),
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.Cancel,
            QMessageBox.StandardButton.Cancel,
        )
        if answer == QMessageBox.StandardButton.Yes:
            self.client.start_selective_restore(project_id, snapshot_id, selected_path)

    def apply_backup_status(
        self,
        available,
        repository_initialized,
        repository_unlocked,
        busy,
        phase,
        project_id,
        bytes_done,
        total_bytes,
        files_done,
        total_files,
        last_snapshot,
        restore_path,
        last_error,
    ):
        self.backup_engine_status = {
            "available": available,
            "repository_initialized": repository_initialized,
            "repository_unlocked": repository_unlocked,
            "busy": busy,
            "phase": phase,
            "project_id": project_id,
            "bytes_done": bytes_done,
            "total_bytes": total_bytes,
            "files_done": files_done,
            "total_files": total_files,
            "last_snapshot": last_snapshot,
            "restore_path": restore_path,
            "last_error": last_error,
        }
        label = self._(BACKUP_PHASE_LABELS.get(phase, "Stare restic necunoscuta"))
        details = []
        if total_files:
            details.append(
                self._("{done} din {total} fisiere").format(done=files_done, total=total_files)
            )
        if total_bytes:
            details.append(
                self._("{done} din {total}").format(
                    done=size_text(bytes_done), total=size_text(total_bytes)
                )
            )
        if restore_path:
            details.append(self._("Director restore: {path}").format(path=restore_path))
        self.backup_progress_label.setText(" · ".join([label, *details]))
        self.backup_progress.setVisible(busy and bool(total_bytes or total_files))
        if total_bytes:
            self.backup_progress.setValue(min(1000, round(1000 * bytes_done / total_bytes)))
        elif total_files:
            self.backup_progress.setValue(min(1000, round(1000 * files_done / total_files)))
        else:
            self.backup_progress.setValue(0)

        if not available:
            self.set_backup_notice(
                "Motorul restic nu este instalat. Dupa instalarea pachetului Debian «restic», serviciul il va valida la /usr/bin/restic.",
                "warning",
            )
        elif last_error:
            self.set_backup_notice(ERROR_MESSAGES.get(last_error, "Operatia restic a esuat."), "error")
        elif busy:
            self.set_backup_notice(
                self._(
                    "{phase}. Progresul nu include nume de fisiere; continutul snapshotului este afisat numai la cerere si nu este jurnalizat."
                ).format(
                    phase=label
                ),
                "info",
            )
        elif phase == "backup_complete":
            self.set_backup_notice(
                "Snapshotul a fost creat, iar structura repository-ului a trecut verificarea restic. Poti testa restaurarea intr-un director nou.",
                "success",
            )
        elif phase == "restore_complete":
            self.set_backup_notice(
                "Restaurarea s-a terminat intr-un director nou; continutul local original nu a fost suprascris.",
                "success",
            )
        elif repository_unlocked:
            self.set_backup_notice(
                "Repository-ul disposable este deblocat numai pentru aceasta sesiune. Backupul necesita confirmare explicita.",
                "success",
            )
        else:
            self.set_backup_notice(
                "Deblocheaza repository-ul existent sau initializeaza calea disposable o singura data.",
                "warning",
            )

        if (
            phase == "backup_complete"
            and last_snapshot
            and last_snapshot != self.handled_backup_snapshot
            and self.active_backup_digest
            and self.active_backup_digest[0] == project_id
        ):
            self.handled_backup_snapshot = last_snapshot
            self.pending_backup_verification = self.active_backup_digest
            self.active_backup_digest = None
            self.refresh_backup_projects()
        elif phase == "failed" and self.active_backup_digest:
            self.active_backup_digest = None
        if not repository_unlocked:
            self.snapshot_history_loading = False
            self.snapshot_history_project_id = ""
            self.snapshot_history_refresh_pending = False
            self.history_refresh_snapshot = ""
            self.clear_snapshot_history(
                "Deblocheaza repository-ul restic pentru a vedea snapshoturile proiectului selectat."
            )
        elif (
            phase == "backup_complete"
            and last_snapshot
            and last_snapshot != self.history_refresh_snapshot
        ):
            self.history_refresh_snapshot = last_snapshot
            self.request_snapshot_history(force=True)
        else:
            self.request_snapshot_history()
        self.update_backup_actions()

    def backup_scan_failed(self, message):
        self.backup_scan_running = False
        self.backup_scan_task = None
        self.backup_summary.setText(self._("Monitorizarea necesita atentie"))
        self.set_backup_notice(message, "error")
        self.update_backup_actions()

    def apply_project_statuses(self, statuses):
        selected_id = self.selected_project_id()
        self.project_statuses = list(statuses)
        self.project_tree.clear()
        now = QDateTime.currentSecsSinceEpoch()
        due_count = sum(status.due for status in statuses)
        attention_count = sum(status.state == "attention" for status in statuses)
        for status in statuses:
            if status.last_verified_at:
                backup_text = QDateTime.fromSecsSinceEpoch(status.last_verified_at).toString("dd.MM.yyyy HH:mm")
            else:
                backup_text = self._("Niciodata")
            if status.dirty_since:
                age = elapsed_text(now - status.dirty_since, self._)
                activity = self._("{count} fisiere · {size} · {age}").format(
                    count=status.file_count, size=size_text(status.total_bytes), age=age
                )
            else:
                activity = self._("{count} fisiere · {size}").format(
                    count=status.file_count, size=size_text(status.total_bytes)
                )
            item = QTreeWidgetItem(
                [
                    status.name,
                    self._(PROJECT_STATE_LABELS.get(status.state, "Stare necunoscuta")),
                    activity,
                    backup_text,
                ]
            )
            item.setData(0, Qt.ItemDataRole.UserRole, status.id)
            item.setToolTip(0, status.path)
            tone = PROJECT_STATE_TONES.get(status.state, "warning")
            icon_name = {
                "success": "emblem-checked",
                "info": "dialog-information",
                "warning": "dialog-warning",
                "error": "dialog-error",
            }[tone]
            fallback = {
                "success": QStyle.StandardPixmap.SP_DialogApplyButton,
                "info": QStyle.StandardPixmap.SP_MessageBoxInformation,
                "warning": QStyle.StandardPixmap.SP_MessageBoxWarning,
                "error": QStyle.StandardPixmap.SP_MessageBoxCritical,
            }[tone]
            item.setIcon(1, self.icon(icon_name, fallback))
            self.project_tree.addTopLevelItem(item)
            if selected_id == status.id:
                self.project_tree.setCurrentItem(item)
        self.backup_empty.setVisible(not statuses)
        self.project_tree.setVisible(bool(statuses))
        if not statuses:
            self.snapshot_history_project_id = ""
            self.clear_snapshot_history("Adauga un proiect pentru a vedea istoricul snapshoturilor.")
            self.backup_summary.setText(self._("Niciun proiect configurat"))
            self.tabs.setTabText(self.backup_tab_index, self._("Backupuri"))
            self.set_backup_notice(
                "Monitorizarea este locala. Uploadul ramane blocat pana la configurarea si validarea motorului restic.",
                "warning",
            )
        else:
            summary = self._("{count} proiecte urmarite · {due} necesita backup").format(
                count=len(statuses), due=due_count
            )
            if attention_count:
                summary += self._(" · {count} scanari incomplete").format(count=attention_count)
            self.backup_summary.setText(summary)
            self.tabs.setTabText(
                self.backup_tab_index,
                self._("Backupuri ({count})").format(count=due_count)
                if due_count
                else self._("Backupuri"),
            )
            if attention_count:
                self.set_backup_notice(
                    "Unele proiecte nu au putut fi scanate complet. Niciun backup nu va fi propus pentru ele.",
                    "error",
                )
            elif due_count:
                self.set_backup_notice(
                    "Exista proiecte pregatite pentru backup. Deblocheaza repository-ul restic si alege «Backup acum».",
                    "warning",
                )
            else:
                self.set_backup_notice(
                    "Monitorizarea locala este activa; continutul fisierelor nu este citit in timpul scanarii.",
                    "success",
                )
        self.update_backup_actions()

    def add_backup_project(self):
        directory = QFileDialog.getExistingDirectory(
            self, self._("Alege directorul proiectului"), str(Path.home()), QFileDialog.Option.ShowDirsOnly
        )
        if not directory:
            return
        default_name = Path(directory).name or self._("Proiect")
        name, accepted = QInputDialog.getText(
            self,
            self._("Proiect monitorizat"),
            self._("Numele proiectului:"),
            text=default_name,
        )
        if not accepted:
            return
        hours, accepted = QInputDialog.getInt(
            self,
            self._("Interval backup"),
            self._("Notifica dupa cate ore de modificari?"),
            2,
            1,
            168,
            1,
        )
        if not accepted:
            return
        try:
            self.project_monitor.add_project(name, directory, hours * 60)
        except ProjectError as error:
            QMessageBox.warning(self, self._("Proiect refuzat"), self._(str(error)))
            return
        self.refresh_backup_projects()

    def snooze_backup_project(self):
        project_id = self.selected_project_id()
        if not project_id:
            return
        try:
            self.project_monitor.snooze(project_id, 60)
        except ProjectError as error:
            QMessageBox.warning(self, self._("Amanare nereusita"), self._(str(error)))
            return
        self.refresh_backup_projects()

    def remove_backup_project(self):
        project_id = self.selected_project_id()
        selected = self.project_tree.selectedItems()
        if not project_id or not selected:
            return
        answer = QMessageBox.question(
            self,
            self._("Elimina monitorizarea"),
            self._(
                "Opresti monitorizarea proiectului «{name}»?\n\n"
                "Niciun fisier local sau remote nu va fi sters, dar istoricul acestui ID de proiect "
                "nu va mai fi accesibil din aplicatie dupa eliminarea inregistrarii."
            ).format(name=selected[0].text(0)),
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.Cancel,
            QMessageBox.StandardButton.Cancel,
        )
        if answer != QMessageBox.StandardButton.Yes:
            return
        try:
            self.project_monitor.remove_project(project_id)
        except ProjectError as error:
            QMessageBox.warning(self, self._("Eliminare nereusita"), self._(str(error)))
            return
        self.refresh_backup_projects()

    def set_busy(self, busy):
        self.client_busy = busy
        for button in (self.refresh_button, self.ssh_button, self.log_button):
            button.setEnabled(not busy)
        if busy:
            self.statusBar().showMessage(self._("Verificare in curs..."))
        else:
            self.statusBar().showMessage(self._("Verificare finalizata"))
        self.update_actions()
        self.update_backup_actions()
        if not busy:
            self.request_snapshot_history(force=self.snapshot_history_refresh_pending)
            self.request_snapshot_entries(force=self.snapshot_entries_refresh_pending)

    def update_actions(self):
        status = self.latest_status
        available = status is not None and not self.client_busy and not self.service_busy
        unlocked = bool(status and status["config"] == "unlocked")
        permissions = {
            "unlock": not unlocked and bool(status and status["config"] == "present_not_unlocked"),
            "lock": unlocked, "open": bool(status and status["mount"] == "mounted"),
            "mount": bool(status and status["state"] == "Ready" and status["mount"] == "unmounted" and unlocked),
            "unmount": bool(status and status["mount"] == "mounted" and self.owned_mount),
            "connection": unlocked, "storage": unlocked, "health": True,
        }
        primary = "unlock" if permissions["unlock"] else "mount" if permissions["mount"] else None
        for key, button in self.operation_buttons.items():
            button.setEnabled(available and permissions[key])
            role = "primary" if key == primary else "danger" if key == "unmount" else "secondary"
            self.set_visual_property(button, "buttonRole", role)

    def apply_storage(self, available, total, used, free, reason):
        if available:
            self.values["storage"].setText(
                self._("{used} din {total} | Liber: {free}").format(
                    used=size_text(used), total=size_text(total), free=size_text(free)
                )
            )
            self.storage_progress.setValue(round(1000 * used / total) if total else 0)
            self.storage_progress.setToolTip(
                self._("{percent:.1f}% utilizat").format(
                    percent=(100 * used / total) if total else 0
                )
            )
            self.set_visual_property(self.values["storage"], "metricTone", "info")
            self.statusBar().showMessage(self._("Spatiu cloud actualizat"))
        else:
            self.values["storage"].setText(
                self._(ERROR_MESSAGES.get(reason, "Spatiu cloud indisponibil"))
            )
            self.storage_progress.setValue(0)
            self.storage_progress.setToolTip(self._("Spatiu cloud indisponibil"))
            self.set_visual_property(self.values["storage"], "metricTone", "warning")
        self.client.refresh()

    def apply_operation(self, method, success, code):
        if success:
            self.error_label.hide()
            message = self._(
                "Verificarile locale sunt in regula; configuratia ramane blocata."
                if code == "local_ok_configuration_locked"
                else "Operatie finalizata"
            )
        else:
            message = self._(
                ERROR_MESSAGES.get(code, "Operatia nu a putut fi finalizata. Consulta diagnosticul.")
            )
            self.error_label.setText(message)
            self.error_label.show()
        self.statusBar().showMessage(message)
        if method == "LockConfiguration":
            self.values["storage"].setText(self._("Indisponibil; configuratie blocata"))
            self.storage_progress.setValue(0)
            self.storage_progress.setToolTip(self._("Configuratie blocata"))
            self.set_visual_property(self.values["storage"], "metricTone", "warning")
        if method == "UnlockConfiguration" and success:
            self.client.get_storage()
        if method == "StartProjectBackup" and not success:
            self.active_backup_digest = None
        self.client.refresh()

    def apply_operation_details(self, busy, last_operation, last_error, owned_mount):
        self.service_busy = busy
        self.owned_mount = owned_mount
        self.operation_details = f"operation: {last_operation}\nlast_error: {last_error}\nowned_mount: {owned_mount}"
        operation = self._(
            OPERATION_LABELS.get(
                last_operation, "Operatie necunoscuta" if last_operation else "Nicio operatie"
            )
        )
        self.values["last_operation"].setText(
            self._("{operation} (in curs)").format(operation=operation) if busy else operation
        )
        self.values["last_error"].setText(
            self._(
                ERROR_MESSAGES.get(last_error, "Consulta detaliile din Diagnostic.")
                if last_error
                else "Nicio eroare"
            )
        )
        self.set_visual_property(self.values["last_error"], "metricTone", "error" if last_error else "success")
        self.update_details()
        self.update_actions()
        self.update_backup_actions()

    def apply_mount_activity(self, available, queued, in_progress, errors, out_of_space, reason):
        if available:
            if errors or out_of_space:
                text = self._("Atentie: {errors} erori; coada {queued}; active {active}").format(
                    errors=errors, queued=queued, active=in_progress
                )
            elif queued or in_progress:
                text = self._("In asteptare: {queued}; in curs: {active}").format(
                    queued=queued, active=in_progress
                )
            else:
                text = self._("Niciun upload in asteptare")
        elif reason == "mount_not_owned":
            text = self._("Indisponibil pentru mountul pornit extern")
        elif reason == "drive_not_mounted":
            text = self._("Drive demontat")
        else:
            text = self._(ERROR_MESSAGES.get(reason, "Starea transferurilor nu este disponibila"))
        self.values["transfers"].setText(text)
        tone = "error" if errors or out_of_space else "warning" if queued or in_progress or not available else "success"
        self.set_visual_property(self.values["transfers"], "metricTone", tone)

    def request_unmount(self):
        answer = QMessageBox.warning(
            self,
            self._("Demontare Hetzner Drive"),
            self._(
                "Inchide documentele si aplicatiile care folosesc drive-ul. Aplicatia va verifica de doua ori coada uploadurilor si va refuza demontarea daca starea nu este sigura.\n\nContinui?"
            ),
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.Cancel,
            QMessageBox.StandardButton.Cancel,
        )
        if answer == QMessageBox.StandardButton.Yes:
            self.client.unmount()

    def update_details(self):
        lines = "\n".join(f"{key}: {value}" for key, value in (self.latest_status or {}).items())
        self.details.setPlainText(f"{lines}\n{self.operation_details}")

    def apply_logs(self, lines):
        self.log_view.setPlainText("\n".join(lines))
        self.statusBar().showMessage(self._("Evenimente actualizate"))

    def apply_ssh(self, code):
        self.values["ssh"].setText(self._(SSH_LABELS.get(code, "Stare SSH necunoscuta")))
        self.set_visual_property(self.values["ssh"], "metricTone", "success" if code == "loaded" else "warning")
        self.statusBar().showMessage(self._("Verificare SSH finalizata"))
        self.client.refresh()

    def apply_status(self, status):
        if self.latest_status is None:
            self.error_label.hide()
        self.latest_status = status
        state = status["state"]
        label = self._(STATE_LABELS.get(state, "Stare necunoscuta"))
        diagnostics = status["diagnostic"].split(",")
        if state == "Degraded" and "remote_check_failed" in diagnostics:
            label = self._("Verificarea conexiunii a esuat")
        self.state_label.setText(label)
        self.set_visual_property(self.state_label, "stateTone", STATE_TONES.get(state, "warning"))
        self.values["mount"].setText(
            self._(MOUNT_LABELS.get(status["mount"], "Stare necunoscuta"))
        )
        self.values["ssh"].setText(self._(SSH_LABELS.get(status["ssh"], "Stare necunoscuta")))
        self.values["config"].setText(
            self._(CONFIG_LABELS.get(status["config"], "Stare necunoscuta"))
        )
        self.values["version"].setText(status["version"])
        self.values["mount_path"].setText(status["mount_path"])
        cache = size_text(status["cache_bytes"])
        if not status["cache_complete"]:
            cache = self._("Cel putin {cache} (verificare incompleta)").format(cache=cache)
        self.values["cache"].setText(cache)
        self.set_visual_property(self.values["mount"], "metricTone", "success" if status["mount"] == "mounted" else "info")
        self.set_visual_property(self.values["ssh"], "metricTone", "success" if status["ssh"] == "loaded" else "warning")
        self.set_visual_property(self.values["config"], "metricTone", "success" if status["config"] == "unlocked" else "warning")
        self.set_visual_property(self.values["cache"], "metricTone", "info" if status["cache_complete"] else "warning")
        if status["config"] != "unlocked":
            self.values["storage"].setText(self._("Indisponibil; autentificare neverificata"))
            self.storage_progress.setValue(0)
            self.storage_progress.setToolTip(self._("Autentificare necesara"))
            self.set_visual_property(self.values["storage"], "metricTone", "warning")
        elif "remote_check_failed" in diagnostics:
            self.values["storage"].setText(self._("Indisponibil; verifica din nou conexiunea"))
            self.storage_progress.setValue(0)
            self.storage_progress.setToolTip(self._("Verificarea conexiunii a esuat"))
            self.set_visual_property(self.values["storage"], "metricTone", "error")
        self.update_details()
        self.update_actions()
        timestamp = QDateTime.currentDateTime().toString("HH:mm:ss")
        self.statusBar().showMessage(
            self._("Ultima verificare: {timestamp}").format(timestamp=timestamp)
        )
        self.mode_label.setText(self._("Date simulate") if "demo_data" in diagnostics else "")
        if self.tray:
            self.tray.setToolTip(self._("Hetzner Drive: {state}").format(state=label))
            if state in ("Error", "Degraded") and self.previous_state is not None and state != self.previous_state:
                self.tray.showMessage("Hetzner Drive", label, QSystemTrayIcon.MessageIcon.Warning)
        self.previous_state = state

    def show_error(self, message):
        self.latest_status = None
        self.backup_engine_status = None
        self.service_busy = False
        self.owned_mount = False
        self.operation_details = ""
        self.state_label.setText(self._("Serviciu indisponibil"))
        self.set_visual_property(self.state_label, "stateTone", "offline")
        self.error_label.setText(self._(message))
        self.error_label.show()
        for widget in self.values.values():
            widget.setText(self._("Necunoscut"))
            self.set_visual_property(widget, "metricTone", "warning")
        self.storage_progress.setValue(0)
        self.storage_progress.setToolTip(self._("Serviciu indisponibil"))
        self.details.clear()
        self.log_view.clear()
        self.mode_label.clear()
        self.snapshot_history_loading = False
        self.snapshot_history_project_id = ""
        self.snapshot_history_refresh_pending = False
        self.clear_snapshot_history("Serviciul nu este disponibil pentru citirea istoricului.")
        if self.tray:
            self.tray.setToolTip(self._("Hetzner Drive: serviciu indisponibil"))
        self.statusBar().showMessage(self._("Verificare nereusita"))
        self.update_actions()
        self.update_backup_actions()

    def closeEvent(self, event: QCloseEvent):
        if self.tray and QSystemTrayIcon.isSystemTrayAvailable():
            self.hide()
            event.ignore()
        else:
            event.accept()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--tray", action="store_true")
    parser.add_argument(
        "--language",
        choices=("auto", *SUPPORTED_LANGUAGES),
        default=os.environ.get("HETZNER_DRIVE_LANGUAGE", "auto"),
        help="Interface language: auto, Romanian (ro), or English (en)",
    )
    args = parser.parse_args()
    if os.geteuid() == 0:
        parser.error("Run the application as a normal desktop user, never root")
    app = QApplication(sys.argv[:1])
    app.setApplicationName("Hetzner Drive")
    app.setOrganizationName("HetznerDriveManager")
    single_instance = SingleInstance()
    if not single_instance.acquire():
        return 0
    window = DriveWindow(language=resolve_language(args.language))
    single_instance.activate_requested.connect(window.reveal)
    if not args.tray or not window.tray:
        window.show()
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(main())
