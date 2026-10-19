# Validare StrajaBackupSQL 0.1.4

## Actualizare 0.1.4 — iconiță agent

- Iconiță nativă Shell_NotifyIconW, resursa proprie din StrajaBackupSQL-Agent.exe, meniu cu deschidere/Activitate/oprire după backup, tooltip de stare. Fereastră ascunsă și message loop separat de worker; recuperare TaskbarCreated și reîncercare la indisponibilitatea temporară a shell-ului.
- O singură fereastră GUI per profil; o nouă lansare transmite cererea de deschidere/Activitate către fereastra existentă și solicită restaurarea/focusul ei.
- Test nativ agent într-un profil temporar: Windows a acceptat iconița, Shell_NotifyIconGetRect a returnat dreptunghiul ei, nu s-a creat un al doilea agent, cererea Activitate a fost transmisă fără o fereastră suplimentară, iar iconița/fereastra ascunsă au dispărut la oprire.
- Sandbox-ul a refuzat inițial comunicarea cu taskbar-ul (NIM_ADD=false). Același test a trecut cu acces la desktop, fără modificarea preferințelor Windows. Acest test necesită un shell Explorer disponibil și acces la el.
- Testul cu export simulat confirmă că oprirea cerută din tray păstrează procesul activ până când backupul este finalizat și verificat. Nu s-au contactat servere SQL reale.
- Tooltip Unicode valid și trunchiere fără separarea perechilor surrogate; mesajele deschidere/oprire sunt limitate la profilul de test.
- Nu s-au verificat manual poziția vizuală aleasă de Windows, clickurile sau revenirea după un restart real de Explorer; nu s-a repornit Explorer. Installerul nu a fost rulat pe stația utilizatorului.

## Actualizare 0.1.3 — 19 octombrie 2026

- 20 teste standard plus testul complet `backup_pipeline`: trecute. Clippy cu `-D warnings`: trecut. Crearea fizică pe Windows a fișierelor normalizate și cu nume care diferă numai prin majuscule este verificată în directoare temporare.

- Explorator: extindere/restrângere, selecție bază, tabele/vederi cu schemă și tip, filtru și paginare. Teste pentru cache, restrângere în timpul descoperirii și răspunsuri asincrone întârziate.
- Metadate: comenzi SELECT fixe pentru MariaDB/MySQL și PostgreSQL; baza este transmisă separat, nu interpolată în SQL. Numele schemelor/tabelelor sunt transportate hex UTF-8 pentru delimitatori și Unicode. Verificate cazurile cu ghilimele, tab și diacritice.
- Progres: volum exportat, etape, procente de citire/comprimare, actualizare la 500 ms în agent și citire la 2 s în UI. Stocare separată în SQLite fără modificarea schemei existente. Exportul are indicator de activitate, nu un procent inventat.
- Nume: `numele-bazei.sql[.gz]`, normalizare Windows, sufixe pentru coliziuni; verificare strictă a manifestului v2 și compatibilitate cu manifestele v1. Testele resping traversarea directoarelor.
- Test complet fără SQL: un executabil-fixtură local simulează exportul progresiv și răspunsurile cu tabele; verifică GZIP, hash-uri, nume, persistența progresului, curățarea fișierelor temporare, retenția și păstrarea copiei bune după eșec.
- Testele cu servere SQL reale rămân omise. Nu s-au folosit credențiale sau servere ale utilizatorului. Noua interfață este verificată prin compilare și teste de stare; nu s-a făcut inspecție vizuală interactivă.
- Installerul 0.1.3 este destinat stației separate de test; nu este instalat pe stația curentă.


Actualizarea 0.1.1 separă executabilele `StrajaBackupSQL.exe` și `StrajaBackupSQL-Agent.exe`, cu metadate Windows distincte. Installerul distribuie ambele, iar pornirea automată folosește exclusiv agentul SQL. AppId-ul și directorul configurației din versiunea 0.1.0 sunt păstrate. Testele de agent și installer verifică noul executabil, metadatele, intrarea de pornire și curățarea la dezinstalare. Testele SQL de integrare de mai jos au fost executate pentru motorul neschimbat din 0.1.0.

Data: 18 octombrie 2026. Windows x64, Rust 1.99.1, Iced 0.14, Inno Setup 7.1.0.

## Validare 0.1.2 — utilitare incluse

- `cargo test --all-targets --locked`: 14 teste trecute, trei teste SQL cu servere temporare omise intenționat. Niciun server SQL nu a fost pornit sau contactat pentru această actualizare.
- `cargo clippy --all-targets --locked -- -D warnings`: trecut, fără avertismente Rust.
- Test de rezolvare: utilitarele implicite provin exclusiv din directorul executabilului; lipsa lor produce eroare fără căutare în PATH. Căile personalizate absolute și compatibilitatea configurațiilor vechi sunt păstrate; căile relative sunt respinse.
- Arhive oficiale cu SHA-256 fixat: MariaDB 11.4.10, MySQL 8.4.11, PostgreSQL 18.6-5. MariaDB verificat și cu SHA-256 publicat de furnizor; MySQL comparat și cu MD5 publicat. SHA-256 EDB calculat din arhiva descărcată prin HTTPS și fixat pentru reproducere.
- Kit: exact șase executabile SQL client, 120 fișiere în manifestul utilitarelor (aprox. 73 MiB). Fără executabile de server. Licențe originale incluse; sursele MariaDB/MySQL păstrate separat în `dist/sources`.
- Importurile PE normale și întârziate sunt verificate recursiv: fiecare DLL nestandard Windows trebuie să existe local. Runtime-ul Microsoft este inclus atât lângă clienți, cât și lângă GUI și agent.
- Toate cele șase comenzi `--version` trecute cu mediu minimal, PATH limitat la System32 și opțiuni explicite pentru module/charset-uri. Raport: `test-output/bundled-tools.json`.
- Installerul 0.1.2 este compilat pentru testare pe altă stație. Nu a fost instalat/dezinstalat pe stația curentă. Verificarea completă pe un Windows curat și backupul cu noile versiuni de client rămân de efectuat pe stația de test.
- Formularul elimină alegerea obligatorie a folderului bin; opțiunea personalizată este avansată. Nu a fost reinspectat vizual în această actualizare.

## Rezultate anterioare — 0.1.0 / 0.1.1

- `cargo clippy --all-targets --locked -- -D warnings`: trecut, fără avertismente Rust.
- `cargo test --all-targets --locked`: 13 teste trecute. Cele trei teste cu servere reale sunt marcate `ignored` în rularea implicită și au fost executate separat, explicit, cu succes.
- MariaDB 10.5.5: cluster temporar local, descoperire baze, export cu utilizator dedicat, GZIP, verificare, restaurare într-o bază temporară distinctă și compararea conținutului. Trecut.
- MySQL 8.4.3: același flux, inclusiv autentificare `caching_sha2_password`. Trecut.
- PostgreSQL 17.4: cluster temporar local, cont cu SELECT, export cu date Unicode, GZIP, verificare, restaurare într-o bază distinctă, compararea a 200 rânduri prin hash agregat și verificarea păstrării copiei după un export eșuat. Trecut.
- Agent real într-un profil temporar: execuție din coada persistentă, un singur agent, eșec de backup înregistrat înainte de eroarea emailului, oprire controlată și eliminarea heartbeatului. Trecut.
- SMTP: server local simulat fără STARTTLS; clientul a refuzat continuarea și nu a transmis credențiale sau conținut. Trecut.
- Retenție: limita de copii/zile, păstrarea celei mai noi copii, a directoarelor incomplete, a fișierelor suplimentare și a unui alt plan. Trecut.
- Integritate: coruperea GZIP și manipularea numelor din manifest sunt detectate. Trecut.
- Programare: intervale, DST de primăvară/toamnă în Europe/Bucharest, sfârșit de lună, dată unică trecută și dată unică îndepărtată. Trecut.
- DPAPI: criptare și decriptare sub contul curent. Trecut.
- Installer/uninstaller: instalare silent într-un folder al proiectului, AppId de test distinct, verificarea intrării Installed Apps și a hashului executabilului, dezinstalare, eliminarea înregistrării și păstrarea unui fișier de backup exterior instalării. Trecut. Raport: `test-output/installer/result.json`.
- Interfață: ecranul principal și navigarea spre Activitate au fost observate într-o fereastră reală. Formularul complet și installerul interactiv nu au putut fi inspectate până la capăt: sesiunea Windows a fost blocată de utilizator. Grafica installerului a fost inspectată separat.

## Limite de validare și distribuție

Nu au fost folosite adresele serverelor din imaginile de referință. Nu au fost modificate baze de date sau servicii SQL ale utilizatorului. Toate scrierile SQL de pregătire/restaurare au avut loc exclusiv în serverele temporare create de teste.

Nu s-a trimis email către un destinatar real și nu este verificată livrarea cu un furnizor SMTP real. Nu au fost testate servere remote cu TLS/CA propriu, conexiuni UNC, volume foarte mari, opriri de curent sau toate combinațiile de versiuni SQL. Testele de integrare au folosit TCP pe localhost, fără TLS.

Aplicația verifică integritatea, nu execută restaurări automate de probă. Kitul 0.1.2 include utilitarele SQL; testele de restaurare de mai sus folosesc versiunile anterioare de client, nu înlocuiesc validarea celor incluse acum. Agentul rulează în sesiunea utilizatorului, fără Windows Service. Nu există semnare digitală, auto-update sau criptarea arhivelor SQL/GZIP. Compilatorul Inno Setup local raportează utilizare necomercială; pentru ambalare comercială trebuie configurată licența corespunzătoare.

Previzualizarea nu trebuie prezentată drept produs validat pentru producție fără testarea configurațiilor țintă și a recuperării datelor.
