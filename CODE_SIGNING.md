# Code signing policy

StrajaBackupSQL is a separate project from Straja Backup for files, maintained
by [Weblike PRO](https://weblike.ro/) in
[weblikero/straja-backup-sql](https://github.com/weblikero/straja-backup-sql).
Its own code is MIT-licensed; bundled components retain their upstream licenses.

## Current status

Preparation for a SignPath Foundation application, 2026-10-20. No acceptance or
production signing is claimed. Existing Windows downloads are unsigned.

The proposed committer, reviewer and signing approver is
[weblikero](https://github.com/weblikero). MFA and provider role assignments
must be confirmed during onboarding. Production signing requires a deliberate
approval by the designated maintainer.

## Intended scope

Request signatures for `StrajaBackupSQL.exe`, `StrajaBackupSQL-Agent.exe` and the
Windows installer. Establish a Windows CI build with verifiable origin, then
integrate the provider's assigned signing policy. Verify the installer and
uninstaller, test the signed kit, and calculate final checksums after signing.
Never replace previously published files with different bytes under the same tag.

MariaDB, MySQL, PostgreSQL, Rclone and runtime components retain their provenance
and licenses. Do not sign upstream executables individually as Straja-owned code.
Review bundled component eligibility with the provider.

## Conditional attribution

Only after acceptance and successful integration, use:

> Free code signing provided by SignPath.io, certificate by SignPath Foundation

This is a proposed attribution, not a claim that SignPath currently signs this
project. The Foundation's certificate identifies the Foundation as publisher.

See the [desktop privacy policy](PRIVACY.md),
[application preparation](SIGNPATH_APPLICATION.md) and
[SignPath conditions](https://signpath.org/terms).
