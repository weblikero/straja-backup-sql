# StrajaBackupSQL — Database Backup Studio

Windows x64 database backup application written in Rust, with an Iced desktop interface. Supports MariaDB, MySQL and PostgreSQL. Version **0.1.7 is a preview**, intended for testing on a separate Windows station before public distribution at **strajabackup.com**.

## Languages

English is the default. Choose **English, Deutsch, Español, Português (Portugal), Français or Italiano** using the sidebar selector or **Settings & notifications → Language**. Changes apply immediately to the interface and are picked up by the running agent, tray menu and new email notifications. The preference persists between sessions.

The installer offers the same six languages, with English selected on a fresh installation. Its choice becomes the application's initial language; an existing saved application preference takes precedence. Upgrades retain the previous installer language. Offline guides in all six languages are included in `docs/Guide.*.txt`.

Connection names, database identifiers, file names, existing plans and passwords are never translated or migrated. Existing log details keep their recorded language. External SQL-tool, Windows and library diagnostics remain verbatim. New plans default to **UTC**; choose an IANA zone such as `Europe/Berlin` or `America/New_York`. Existing plans retain their time zones. Dates use ISO format and schedule times use a 24-hour clock.

## Install and start

Run `StrajaBackupSQL-0.1.7-windows-x64-setup.exe`, or use the complete portable folder `dist/StrajaBackupSQL`. Setup includes uninstall support, an optional desktop shortcut and optional agent startup at Windows sign-in. Installation is per user, without administrator privileges.

1. Add a connection: name, engine, server, port, username and password. SQL client utilities are bundled and selected automatically. Advanced options allow a compatible custom client folder. Existing custom paths remain unchanged on upgrade.
2. Keep certificate and server identity verification enabled for TLS. Supply a CA certificate when needed. There is no automatic fallback to unverified TLS.
3. Select **Test / Explore**. Click a connection to expand or collapse it; click a database to see tables and views, with schema, object type, search and pagination. Only metadata is read. MySQL/MariaDB system databases are hidden. PostgreSQL discovery requires access to the `postgres` maintenance database.
4. Create a plan with explicitly selected databases, destination, schedule, compression and retention. Databases created later on the server are not automatically added.
5. Select **Back up now**, then monitor **Activity**. Open the backup folder or verify it again. Copies removed by retention remain in history but can no longer be opened or verified.
6. Start the agent in Settings. Configure and save SMTP, send a test email, and enable failure notifications separately for each desired plan.

## Product identity and local data

**StrajaBackupSQL** is the SQL product in the StrajaBackup suite, separate from **StrajaBackup** for Windows files. The executables are `StrajaBackupSQL.exe` and `StrajaBackupSQL-Agent.exe`; the Windows description is **StrajaBackupSQL Agent**. Keep both in the same folder. The startup entry is `StrajaBackupSQL.Agent`. No Windows service is currently installed.

The installer AppId stays unchanged for upgrades. Configuration remains in `%LOCALAPPDATA%\StrajaSQL\straja-sql.sqlite`, preserving compatibility with previous SQL releases. This is not shared with the file-backup product. Passwords use Windows DPAPI for the same Windows user. A blank password field preserves the saved password; recreate the connection to replace a saved password with an empty one.

Before upgrading or uninstalling, close the window and stop the agent after the current backup finishes. Uninstall removes application files, private utilities, shortcuts and the startup entry, while keeping settings and backups. Store backups outside the installation folder.

## Software updates

Starting with 0.1.7, the open app checks for updates when due at startup and every 24 hours. **Settings & notifications → Software updates** also offers a manual check and an automatic-check toggle. A banner offers the official release page; you can skip a version. Downloading and running the installer is your choice. Update failures do not affect backups.

The app checks `https://strajabackup.com/updates/strajabackupsql/windows-x64/stable.json`, falling back to stable public GitHub releases in [weblikero/straja-backup-sql](https://github.com/weblikero/straja-backup-sql). Product, platform, channel, semantic version and release-page URL are validated. Other suite products, drafts and prereleases are excluded. No GitHub account or token is needed by end users.

The tray agent alone does not check updates. Earlier app versions require one manual upgrade to obtain this feature. See [docs/UPDATES.en.txt](docs/UPDATES.en.txt) for publishing instructions, the JSON schema, privacy details and limitations. The repository currently contains preview source; it does not imply a tested production installer is available.

## Bundled utilities

The offline package includes MariaDB 11.4.10 (`mariadb.exe`, `mariadb-dump.exe`), MySQL 8.4.11 (`mysql.exe`, `mysqldump.exe`) and PostgreSQL 18.6 (`psql.exe`, `pg_dump.exe`), with required libraries, authentication modules and app-local Visual C++ runtime DLLs. Files live under `tools/<engine>/bin`, relative to the executables.

No SQL servers are installed or started. Existing SQL installations, services, configuration and PATH are not changed. No global Visual C++ installer runs. Bundled versions do not guarantee compatibility with every server; choose compatible tools in advanced options when necessary.

Provenance and SHA-256 hashes are in `tools/manifest.json`. Original licenses accompany each client; see `SQL-TOOLS-NOTICES.txt` and `THIRD-PARTY-NOTICES.txt`. Corresponding MariaDB/MySQL sources are prepared in `dist/sources` for distribution alongside the GPL utilities.

## Export, verification and retention

The app has no SQL editor, server restore, database deletion or maintenance command. Discovery uses fixed read-only metadata queries. Export uses official dump tools without a shell or arbitrary user-supplied command options. Use a dedicated account with minimum read/export privileges; the app does not grant privileges.

MySQL/MariaDB use `--single-transaction --quick --skip-lock-tables --no-tablespaces`, including routines, events and triggers. MySQL disables GTID export and column statistics. Consistency applies to transactional tables such as InnoDB; nontransactional engines need a separate strategy. Avoid schema changes during export. PostgreSQL uses `pg_dump --format=plain` with `default_transaction_read_only=on`. Databases are exported sequentially without a common cross-database snapshot. Global PostgreSQL roles are not exported.

Runs create `straja-plan-<UUID>/partial-<UUID>`. Export files preserve database names, for example `shop.sql.gz`. Names incompatible with Windows are sanitized with stable suffixes; case-insensitive collisions are disambiguated. The exact SQL name remains in manifest version 2. Legacy version 1 backups using `database-001.sql` remain supported and are not renamed.

Verification checks process exit status, nonempty output, completion markers, reread SHA-256 and decompressed GZIP content. After success, the folder is atomically renamed to `backup-<UUID>` on the same volume. **File integrity does not prove a full SQL restore will succeed.** Regularly test restoration on a separate server with official tools. SQL/GZIP backups are not encrypted.

Activity displays phase, database, processed size and elapsed time. Dump tools do not expose a reliable total size, so export shows activity and volume without a fabricated percentage. Compression and verification show measured progress for the current phase; a new phase or database restarts that percentage. Only successful completion is final 100%.

Retention limits count and/or age; blank means unlimited. Exceeding either limit expires a copy. The newest copy is always kept. Retention runs only after a verified complete plan and revalidates candidates. It deletes only ordinary files listed in owned manifests and empty directories, refusing links, reparse points, unexpected files and folders outside the plan. Corrupt, unknown and partial backups are preserved for inspection and may require manual cleanup. Retention rereads data and can take time on large backups.

## Scheduling and tray agent

Schedules support manual, minutes, hourly, daily, weekly, monthly, yearly and once. Monthly days 29–31 adjust to the last available day. Nonexistent daylight-saving times are skipped; repeated times run once. Hourly is 60 minutes after saving or a scheduled run, not necessarily the top of the hour.

The agent checks the durable SQLite queue every two seconds and runs one plan at a time. A profile-specific file lock prevents duplicate agents. After downtime it catches up one overdue run per plan. An interrupted run becomes **Interrupted** on restart. Failed backups and SMTP deliveries are not retried immediately. The time limit applies per database export, not to compression or verification.

The agent owns the icon beside the Windows clock; check the hidden **^** menu. Click to open or restore the existing window. Right-click for **Open StrajaBackupSQL**, **Backup activity**, or **Stop agent after current backup**. The tooltip shows the current database/phase, available percentage or recent failure. Closing the window leaves the agent running. Graceful stop waits for the active backup. Explorer restarts trigger icon registration again; temporary shell unavailability does not stop backups. The app does not change taskbar preferences.

The agent runs in the signed-in user session. It does not run after sign-out, wake the PC, or back up while Windows is asleep or powered off.

## Email and credentials

SMTP requires STARTTLS or implicit TLS with verified certificates. Username/password or app-password authentication is supported; OAuth and multiple recipients are not implemented. Backup failure is persisted before attempting email. Mail failure is recorded in run details. Test messages are sent only on request; successful backups do not generate automatic mail. Forced termination can prevent notification delivery.

SQL passwords are passed only in the child process environment (`MYSQL_PWD` / `PGPASSWORD`), never in command arguments or password files. Processes with sufficient access to the same Windows account may inspect that environment. DPAPI protects stored credentials, not a compromised account. Credentials are not sent to unrelated services.

## Build and validation

Requires Windows x64, recent stable Rust supporting `File::try_lock`, Visual Studio C++ Build Tools and Windows SDK. Packaging also needs Python 3.12+ and Inno Setup 7. The Cargo wrapper can reuse the sibling project's toolchain without changing its source. SQL archives are pinned by SHA-256 in `scripts/sql-tools.lock.json` and cached in `.vendor-cache`.

```powershell
.\scripts\cargo.ps1 test --locked --all-targets
.\scripts\cargo.ps1 -CargoArgs @('clippy','--locked','--all-targets','--','-D','warnings')
python .\scripts\locales.py
.\scripts\package.ps1 -DownloadTools
python .\scripts\bundle_tools.py --verify-only .\dist\StrajaBackupSQL\tools
```

English catalog keys use explicit numbered placeholders; user values are never translated or reinterpreted. Edit `locales/catalog.json` and `installer/translations.json`; `scripts/locales.py` validates coverage and placeholder parity and generates the guides and installer text. Stable persisted statuses and phases have a separate presentation map for backward compatibility.

Tests cover translations, Unicode/placeholders, language persistence, constructing UI pages in every language, scheduling/DST, SQL command safety, names, integrity, retention and queue behavior. The backup pipeline uses fake clients without real SQL servers. The native tray test requires an interactive Windows desktop and uses a temporary profile; it also checks live agent language changes. Real SQL round-trip tests are ignored by default and must only run on a separate test station.

`scripts/package.ps1 -SkipBuild -TestVariant` creates a separate test installer AppId. Do not use this variant for normal installation.

## Release status

Version 0.1.7 adds product-scoped update discovery, in-app notifications and update settings in all six languages. See `docs/VALIDATION-0.1.7.txt` for current coverage. The previous isolated installer execution test stalled before file copying/registration on the development workstation; its cause remains unresolved. Installation and uninstall are not validated. `docs/TESTING.en.txt` describes the separate-station checks.

This preview is unsigned. Clean-machine installation/upgrade/uninstall, real-server restore tests and visual review of all six languages at supported display scales remain to be completed on the test station. Before public publication, arrange publisher signing and resolve distribution licensing. The local Inno compiler reports a noncommercial license; review the appropriate compiler and dependency licenses for your release. Building does not upload or deploy the website.
