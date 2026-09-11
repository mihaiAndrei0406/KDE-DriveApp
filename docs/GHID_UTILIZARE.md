# Ghid de utilizare — Hetzner Drive Manager

Acest document descrie comportamentul disponibil in aplicatia curenta. Va fi
actualizat odata cu fiecare functionalitate noua vizibila utilizatorului.
Functionalitatile doar propuse sunt enumerate separat si nu trebuie presupuse ca
fiind active.

## Limba interfetei

Aplicatia foloseste romana cand limba sesiunii desktop incepe cu `ro` si engleza
pentru celelalte locale. Limba poate fi aleasa explicit la pornire cu
`hetzner-drive --language ro` sau `hetzner-drive --language en`. Variabila
`HETZNER_DRIVE_LANGUAGE` accepta aceleasi valori. Dialogurile securizate pinentry
sunt bilingve deoarece backendul poate fi pornit separat, la cerere, prin D-Bus.

Selectarea limbii schimba textele GUI, erorile locale, notificarile si starile
afisate; codurile tehnice D-Bus si continutul evenimentelor sanitizate raman
neschimbate pentru compatibilitate si depanare.

## Pornire rapida pentru lucru zilnic

1. Deschide `Hetzner Drive` din meniul de aplicatii KDE. Backendul user-systemd
   este pornit la cerere prin D-Bus. Aplicatia nu monteaza automat drive-ul.
2. In pagina `Stare`, verifica daca cheia SSH este incarcata. Daca este necesar,
   foloseste `Verifica SSH` dupa ce cheia a fost incarcata prin fluxul keychain.
3. Apasa `Deblocheaza` si introdu in pinentry parola care protejeaza fisierul
   `rclone.conf`.
4. Pentru acces ca filesystem, apasa `Monteaza`, apoi `Deschide`. Pentru backupul
   proiectelor locale nu este necesara montarea drive-ului.
5. Pentru backup, deschide `Backupuri`, apasa `Deblocheaza restic` si introdu
   parola separata a repository-ului restic.
6. La final, demonteaza drive-ul numai dupa inchiderea documentelor si dupa ce
   aplicatia raporteaza ca transferurile sunt sigure. Foloseste `Blocheaza restic`
   si/sau `Blocheaza` daca vrei eliminarea credentialelor din sesiunea backendului.

## Ce parola se introduce

| Dialog sau actiune | Valoarea corecta |
| --- | --- |
| `Deblocheaza` configuratia | Parola care protejeaza fisierul criptat `rclone.conf` |
| `Initializeaza restic` | O parola restic noua, separata, introdusa identic de doua ori |
| `Deblocheaza restic` | Parola restic salvata in managerul de parole |
| Confirmare backup/restore | Butonul explicit de confirmare din pinentry; nu este o parola |
| Cheie SSH | Passphrase-ul este gestionat de keychain/agent, nu de aceasta aplicatie |

Parola crypt si saltul crypt sunt configurate in interiorul configuratiei rclone
si nu se introduc in dialogurile aplicatiei. Nu copia parole in terminal, chat,
capturi de ecran sau loguri.

## Pagina Stare

Pagina principala afiseaza:

- starea generala: blocat, pregatit, montat, degradat sau eroare;
- daca drive-ul este montat la `$HOME/HetznerDrive`;
- prezenta cheii SSH dedicate si starea configuratiei rclone;
- versiunea rclone, spatiul cloud si spatiul alocat cache-ului local;
- activitatea VFS observabila pentru mounturile pornite de aplicatie;
- ultima operatie si ultima eroare inregistrata.

Actiunile disponibile sunt:

- `Actualizeaza`: reciteste starile locale si backendul;
- `Verifica SSH`: verifica fingerprintul exact in agent;
- `Deblocheaza` / `Blocheaza`: gestioneaza parola configuratiei in sesiune;
- `Monteaza` / `Demonteaza`: controleaza numai mountul pornit de aplicatie;
- `Deschide`: deschide calea fixa in Dolphin;
- `Conexiune`: verifica remote-ul autentificat;
- `Spatiu cloud`: citeste capacitatea raportata de remote;
- `Diagnostic`: combina verificarile locale si, cand este deblocat, conexiunea.

Un mount observat nu este dovada unui backup finalizat. Pentru protectie foloseste
snapshoturile din pagina `Backupuri`.

## Montarea si demontarea drive-ului

### Montare

1. Inchide orice mount extern existent pe aceeasi cale.
2. Verifica SSH si deblocheaza configuratia rclone.
3. Apasa `Monteaza`. Aplicatia refuza o cale ocupata, un al doilea mount al
   remote-ului sau directoare/cache cu stare nesigura.
4. Dupa starea `Montat`, apasa `Deschide` pentru Dolphin.

### Demontare sigura

1. Salveaza si inchide toate documentele care folosesc drive-ul.
2. Verifica sectiunea `Transferuri`; nu trebuie sa existe uploaduri active, in
   asteptare, erori sau lipsa de spatiu.
3. Apasa `Demonteaza` si confirma avertismentul. Backendul verifica de doua ori
   activitatea si refuza demontarea daca nu o poate dovedi sigura.

Nu sterge manual cache-ul VFS pentru depanare: poate contine scrieri care trebuie
reluate. `Blocheaza` nu demonteaza un drive deja montat si nu retrage fisierele
plaintext deja accesibile prin FUSE.

## Proiecte monitorizate

1. Deschide `Backupuri` si apasa `Adauga proiect`.
2. Alege un director local obisnuit, detinut de utilizatorul curent, care nu este
   un symlink si nu contine/nu se afla in mountul Hetzner.
3. Alege numele si intervalul de notificare, intre 1 si 168 de ore. Valoarea
   implicita este doua ore; perioada de liniste este zece minute.
4. Aplicatia scaneaza metadatele la aproximativ cinci minute si la comanda
   `Scaneaza acum`. Nu deschide continutul fisierelor in aceasta etapa.

Scanarea urmareste nume relative, dimensiuni, moduri si timpi de modificare, nu
urmeaza symlinkuri si se opreste la 100.000 de intrari sau 64 de niveluri.
Directoare generate precum `target`, `node_modules`, `.venv`, `build`, `dist` si
cache-uri uzuale sunt excluse implicit.

Starile proiectului sunt:

- `Backup initial necesar`: nu exista inca un snapshot verificat pentru digest;
- `Modificari detectate`: proiectul s-a schimbat, dar intervalul nu a expirat;
- `Backup recomandat`: intervalul si perioada de liniste au expirat;
- `Amanat`: recomandarea a fost amanata o ora;
- `Verificat`: ultimul digest observat a fost salvat si verificat;
- `Scanare incompleta`: calea ori limitele de siguranta necesita atentie.

`Elimina` scoate numai intrarea din registrul local. Nu sterge directorul local,
snapshoturile remote sau restaurarile, dar istoricul acelui ID nu va mai fi
accesibil din aplicatie dupa eliminare. Restaureaza ce ai nevoie inainte de a
elimina proiectul.

## Repository-ul restic

Repository-ul acestei faze are calea fixa:

`hetzner-crypt:HetznerDrive-Backup-Disposable/restic-v1`

### Initializare — o singura data

1. Deblocheaza mai intai configuratia rclone.
2. In `Backupuri`, apasa `Initializeaza restic` si citeste avertismentul.
3. Alege o parola restic lunga si separata. Introdu-o identic in cele doua
   dialoguri pinentry si salveaz-o intr-un manager de parole.
4. Dupa succes, starea devine `Repository restic pregatit`.

Nu repeta initializarea pentru repository-ul existent. Dupa restart foloseste
`Deblocheaza restic`. O incercare de initializare peste un repository existent
trebuie sa esueze fara a-l inlocui.

### Backup manual verificat

1. Deblocheaza configuratia rclone si repository-ul restic.
2. Selecteaza proiectul. `Backup acum` ramane dezactivat daca scanarea este
   incompleta, proiectul nu este sigur ori alt job ruleaza.
3. Apasa `Backup acum`, citeste avertismentul si confirma explicit in pinentry.
4. Urmareste contoarele de fisiere si bytes. Numele fisierelor nu sunt afisate in
   progres sau loguri.
5. Succesul inseamna ca restic a returnat un ID complet de snapshot si ca
   `restic check --with-cache` a trecut. Proiectul este marcat `Verificat` numai
   daca digestul local nu s-a schimbat in timpul jobului.

Exista un singur job restic simultan. Backendul este append-only si aplicatia nu
ofera `forget`, `prune`, delete sau alegerea altui remote.

### Recomandari si notificari

Aplicatia recomanda backup dupa intervalul configurat si zece minute fara alte
modificari observate. Recomandarea ramane vizibila in tab chiar daca KDE suprima
notificarea tray. `Amana o ora` amana doar notificarea; nu creeaza un snapshot.
Backupul nu porneste automat in versiunea curenta si cere intotdeauna confirmare
explicita in pinentry.

## Restaurarea unui snapshot

1. Deblocheaza configuratia rclone si repository-ul restic, apoi selecteaza
   proiectul dorit. Aplicatia incarca istoricul direct din repository; montarea
   drive-ului nu este necesara.
2. In `Istoric snapshoturi`, alege versiunea dupa data, numarul de fisiere,
   dimensiune si prefixul ID-ului. Lista este ordonata de la cel mai nou la cel
   mai vechi, primul element fiind selectat implicit. `Actualizeaza istoricul`
   reciteste repository-ul.
3. Apasa `Restaureaza snapshot`, citeste avertismentul si confirma in pinentry.
4. Aplicatia restaureaza numai snapshotul selectat, cu verificare, intr-un
   director nou:
   `$HOME/HetznerDrive-Restores/restore-...`.
5. Inspecteaza si compara fisierele. Originalul nu este suprascris.

Sunt afisate cel mult 256 dintre cele mai recente snapshoturi care corespund
simultan tagului aplicatiei, ID-ului proiectului si caii exacte inregistrate.
Backendul accepta pentru restore numai un ID complet obtinut din acest istoric in
sesiunea curenta. La `Blocheaza restic`, lista si autorizarea locala sunt sterse;
dupa restart sau o noua deblocare selecteaza proiectul pentru a le reincarca.

## Sesiune, tray si pornire

- Inchiderea ferestrei o ascunde in tray daca mediul ofera system tray.
- `Exit` din tray inchide GUI-ul, dar nu garanteaza oprirea serviciului sau
  demontarea drive-ului.
- `Blocheaza restic` elimina numai parola restic din memoria backendului.
- `Blocheaza` configuratia elimina si parola rclone, si parola restic; un mount
  existent ramane accesibil pana la demontare.
- Restartul serviciului elimina ambele parole. Nu exista deblocare automata.
- Integrarea D-Bus porneste serviciul la cerere. Autostartul tray si montarea
  automata sunt dezactivate in configuratia instalata curenta.
- Backendul primeste identitatea publica a contului prin
  `HETZNER_DRIVE_SFTP_HOST` si `HETZNER_DRIVE_SFTP_USER`; installerul cere
  `--sftp-host` si `--sftp-user`, valideaza relatia lor si le scrie numai in
  fisierul privat al serviciului. Valorile trebuie sa corespunda configuratiei
  rclone criptate.
- `uninstall` elimina numai fisierele de integrare administrate si nu cere hostul
  sau utilizatorul Storage Box; fara `--apply` ramane doar o previzualizare.
- Calea implicita a cheii private este `$HOME/.ssh/hetzner_storagebox`. Daca
  `rclone.conf` foloseste alt nume, backendul trebuie pornit cu variabila
  `HETZNER_DRIVE_SSH_KEY` setata la acea cale; installerul accepta aceeasi valoare
  prin `--ssh-key`. Calea trebuie sa fie direct sub `$HOME/.ssh`, iar aplicatia
  accepta in cale numai litere ASCII, cifre si caracterele `/._-`; aplicatia
  citeste numai fisierul public corespunzator cu sufixul `.pub`.

## Evenimente si Diagnostic

`Evenimente` afiseaza numai operatii si coduri sanitizate, nu parole sau nume brute
de fisiere. `Diagnostic` afiseaza valorile tehnice tipizate utile la depanare.
La raportarea unei probleme copiaza codul de eroare, nu parolele si nu continutul
configuratiei.

## Probleme frecvente

- Buton gri: verifica proiectul selectat, SSH, configuratia rclone, restic si daca
  exista deja un job in curs.
- `Configuratie blocata`: apasa `Deblocheaza` si foloseste parola `rclone.conf`.
- Politica Storage Box lipsa/nevalida: reinstaleaza integrarea cu valorile corecte
  `--sftp-host`, `--sftp-user` si, daca este cazul, `--ssh-key`; nu introduce
  parole in aceste argumente.
- `Repository restic blocat`: apasa `Deblocheaza restic` si foloseste parola
  restic din manager.
- Parola restic refuzata: verifica tipul parolei; nu folosi crypt password, salt
  sau parola configuratiei.
- `Scanare incompleta`: verifica daca directorul exista, apartine utilizatorului,
  nu a devenit symlink si nu depaseste limitele.
- Backup creat, dar check esuat: nu considera proiectul verificat; pastreaza datele
  locale si investigheaza conexiunea/repository-ul inainte de alta operatie.
- Istoric indisponibil: verifica daca ambele configuratii sunt deblocate, proiectul
  este selectat si niciun alt job restic nu ruleaza, apoi apasa
  `Actualizeaza istoricul`.
- Snapshot refuzat la restore: istoricul s-a schimbat ori repository-ul a fost
  blocat intre timp; reincarca lista si selecteaza din nou snapshotul.
- Serviciu indisponibil: redeschide aplicatia pentru activare D-Bus; daca problema
  persista, consulta `Evenimente`, `Diagnostic` si `docs/RECOVERY.md`.

## Reguli de siguranta

- Pastreaza separat parolele rclone si restic intr-un manager de parole.
- Nu trata mountul, notificarea sau scanarea drept dovada de backup.
- Nu lucra direct in directorul unei restaurari ca si cum ar fi proiectul original;
  inspecteaza si copiaza controlat ce ai nevoie.
- Nu sterge VFS cache, repository-ul ori snapshoturile pentru depanare.
- Pastreaza cel putin o copie independenta pentru documentele critice. Repository-ul
  disposable nu are inca politica de retentie si nu inlocuieste strategia 3-2-1.

## Functionalitati planificate, dar inactive

- restaurarea unui fisier/subdirector ales;
- automatizare opt-in dupa quiet period, cu reguli de baterie/retea;
- anularea controlata a unui job si recuperarea dupa intrerupere;
- verificari periodice esantionate ale datelor;
- politica de retentie auditata separat.

Niciuna dintre acestea nu trebuie considerata disponibila pana cand ghidul muta
explicit functionalitatea intr-o sectiune de utilizare curenta.
