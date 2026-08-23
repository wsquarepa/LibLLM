# `db` subcommands are read-only by default and `db import` always snapshots

`libllm db sql` and `db shell` open with `PRAGMA query_only = ON` and only lift it under `--write`, because the subcommands exist for inspection and an accidental write through the decryption pipeline is irreversible. `db import` always creates a snapshot before swapping the database file; a `--no-backup` flag was considered and rejected, since the pre-swap snapshot is the only recovery story for a failure between building the replacement and renaming it into place. `db dump` and `db import` refuse to run while another LibLLM process holds the database (probed with `BEGIN IMMEDIATE; ROLLBACK;`), and `db import` refuses a dump whose schema version differs from the storage crate's `CURRENT_VERSION`, so adding a migration automatically tightens the import gate.

Exit codes are shared across the group: `1` generic, `2` user declined, `3` schema-version mismatch, `4` another process holds the database.
