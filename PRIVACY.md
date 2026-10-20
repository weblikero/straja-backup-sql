# StrajaBackupSQL desktop privacy policy

Reviewed 2026-10-20 against the repository's 3.0.7 source. Earlier releases may
have different features. This covers the desktop application, not website
analytics, download counters or contact forms on strajabackup.com or weblike.ro.

StrajaBackupSQL is maintained by Weblike PRO. No Straja account is required.
The reviewed code has no usage-analytics or automatic crash-report upload
feature. The application does not send database contents to Weblike PRO.

## Local information and credentials

Connections, server addresses, database selections, plans, destinations,
schedules, settings, history and diagnostics are stored locally. These records
are not encrypted as a whole. Windows stores data in
`%LOCALAPPDATA%\StrajaSQL`; native platform paths are described in
[the native guide](docs/NATIVE-KITS.en.txt).

Saved passwords use Windows DPAPI or the native credential store on Ubuntu
and macOS. Backup utilities receive SQL passwords in their process environment.
Processes with sufficient access to the same user account or administrator
privileges may inspect them.

SQL dumps and GZIP backups are **not encrypted**. Compression is not encryption.
Protect access to both local and online backup locations.

## Database connections and backup destinations

Testing, exploring and exporting connect to the server you configure. The SQL
server receives authentication and metadata/export queries. Exports read the
selected databases into local backup files; server operators can log access.
The application does not provide server restore or database deletion commands.

If an online destination is configured, Rclone uploads the copy to the chosen
FTP/FTPS server, Google Drive, Dropbox or Amazon S3 account. Verification reads
remote data back, and retention can remove eligible app-owned copies. Providers
receive file contents and metadata, account access and network information.
Google Drive/Dropbox authorization uses a browser consent flow. Plain FTP does
not encrypt the transport; use FTPS when supported.

Relevant policies: [Google](https://policies.google.com/privacy),
[Dropbox](https://www.dropbox.com/privacy),
[AWS](https://aws.amazon.com/privacy/). Consult your database, network-share and
FTP operator for their policies. See [destination details](docs/DESTINATIONS.en.txt).

## Optional email notifications

Test messages are sent when requested. Backup-failure emails are sent only for
plans with notifications enabled, through your configured SMTP server to the
configured recipient. They include the plan name, time and error details.
Errors may reveal database names, server details or filesystem paths. SMTP
requires verified TLS. Mail providers and recipients can retain the messages.
No database dump is attached by this notification feature.

## Update checks and downloads

Automatic checks are enabled by default and can be disabled in
**Settings & notifications → Software updates**. When enabled, the open interface
checks when due at startup and approximately every 24 hours. Manual checks remain
available. The tray agent does not independently check.

The primary endpoint is
`https://api.github.com/repos/weblikero/straja-backup-sql/releases/latest`.
If that source is unavailable or invalid, the app tries the product-specific
feed at `https://strajabackup.com/updates/strajabackupsql/<platform>/stable.json`.

Requests include the product/version User-Agent and normal connection information
such as the public IP address. They do not upload SQL credentials, connection
definitions, database contents, plans or logs. GitHub handles requests under its
[privacy statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement).
The fallback request reaches the Straja website's hosting infrastructure.

User-initiated update downloads contact GitHub and its release asset hosts.
Installation requires a user action; the automatic-check setting alone does not
authorize installation. See [update details](docs/UPDATES.en.txt).

## External links, support and removal

Opening external links uses your browser and the destination's policy. Logs are
not automatically attached to support requests. Review and redact diagnostics
before sharing; the [GitHub issue tracker](https://github.com/weblikero/straja-backup-sql/issues)
is public.

Uninstallation preserves local settings and backups. Retention operates according
to saved plans; removing the application does not delete remote accounts or
revoke provider authorization. Revoke cloud access through the relevant provider.
