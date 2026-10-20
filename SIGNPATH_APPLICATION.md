# SignPath application preparation — StrajaBackupSQL

Separate from the application for Straja Backup (file backups).
Prepared 2026-10-20; not submitted by this preparation.

- Project: StrajaBackupSQL
- Repository: https://github.com/weblikero/straja-backup-sql
- Homepage: https://strajabackup.com/
- Downloads: https://github.com/weblikero/straja-backup-sql/releases
- Privacy: https://github.com/weblikero/straja-backup-sql/blob/main/PRIVACY.md
- Code signing policy: https://github.com/weblikero/straja-backup-sql/blob/main/CODE_SIGNING.md
- Own-code license: MIT, selected by the owner on 2026-10-20.
- Contact details: enter in the provider form; not stored in this public document.

## Tagline

Desktop database backups for MariaDB, MySQL and PostgreSQL.

## Description

StrajaBackupSQL is a database backup application written in Rust and maintained
by Weblike PRO. It helps users configure database connections, select databases,
schedule exports, verify backup-file integrity, manage retention and receive
optional failure notifications. Backups are kept locally and can also be copied
to supported online destinations. We request signing for the Windows desktop
application, background agent and installer.

## Reputation and build disclosure

This is a new project; we do not claim an established user base or an independent
security audit. Public source, release history and validation documentation are
available in the repository. Windows kits are currently unsigned and built
locally. Native Linux/macOS builds use GitHub Actions. A verifiable Windows CI
build and signing integration are still to be implemented.

## Before production signing

- Confirm maintainer MFA and provider account roles.
- Establish Windows CI provenance accepted by SignPath; native CI alone does
  not establish the origin of Windows executables.
- Review bundled SQL utilities, Rclone and runtime redistribution licenses
  against the provider's requirements. Own-code MIT does not relicense them.
- Verify product/version resources and signatures for the GUI, agent, installer
  and uninstaller, then exercise installation and upgrade on a disposable system.
- Link the signing/privacy policies from the website and release pages.
- Review installation-time privacy disclosure and update-check opt-out; the
  current app already has a Settings toggle.
- Include policies and license in newly built packages. Existing downloads
  are not rebuilt or replaced by this documentation change.

Apply at https://signpath.org/apply after reviewing the outstanding work.
Acceptance and timing are determined by SignPath; this document is not proof
of approval. Do not report unfinished work as complete.
