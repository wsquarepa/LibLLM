//! Typed error enum for the backup crate.

use std::path::PathBuf;

use libllm::crypto::CryptoError;

/// All errors produced by the backup crate.
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    // ---- index errors ----
    /// The index file could not be read from disk.
    #[error("failed to read index file {path}: {source}")]
    IndexRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The index file could not be deserialized.
    #[error("failed to parse index file {path}: {source}")]
    IndexParse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    /// The index file could not be serialized.
    #[error("failed to serialize index: {0}")]
    IndexSerialize(#[source] serde_json::Error),

    /// The index file could not be written to disk.
    #[error("failed to write index file {path}: {source}")]
    IndexWrite {
        path: PathBuf,
        #[source]
        source: CryptoError,
    },

    /// The index contains an entry with an unsafe filename (path traversal, absolute path, etc.).
    #[error("backup index contains unsafe filename: {filename}")]
    UnsafeFilename { filename: String },

    /// A backup id referenced in the chain was not found in the index.
    #[error("backup id not found in index: {id}")]
    ChainEntryNotFound { id: String },

    /// A diff entry is missing its `base_id` field.
    #[error("diff entry {id} has no base_id")]
    DiffMissingBaseId { id: String },

    /// A `base_id` reference in the chain could not be resolved.
    #[error("base_id {base_id} referenced by {referencing_id} not found in index")]
    ChainBaseNotFound {
        base_id: String,
        referencing_id: String,
    },

    /// A cycle was detected while walking the backup chain.
    #[error("cycle detected in backup chain at id: {id}")]
    ChainCycle { id: String },

    /// The backup chain depth exceeds the hard cap.
    #[error("backup chain exceeds maximum depth of {max} at id: {id}")]
    ChainTooDeep { max: usize, id: String },

    /// Index path has no parent directory component.
    #[error("index path {path} has no parent")]
    IndexNoParent { path: PathBuf },

    /// Backups dir path has no parent (data_dir) component.
    #[error("backups dir {path} has no parent")]
    BackupsDirNoParent { path: PathBuf },

    // ---- snapshot / file I/O errors ----
    /// The backups directory could not be created.
    #[error("failed to create backups directory: {0}")]
    CreateBackupsDir(#[source] std::io::Error),

    /// A backup file could not be written to disk.
    #[error("failed to write backup file {path}: {source}")]
    WriteBackupFile {
        path: PathBuf,
        #[source]
        source: CryptoError,
    },

    /// A backup file could not be read from disk.
    #[error("failed to read backup file {filename}: {source}")]
    ReadBackupFile {
        filename: String,
        #[source]
        source: std::io::Error,
    },

    /// The backups directory could not be enumerated.
    #[error("failed to read backups directory {path}: {source}")]
    ReadBackupsDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A directory entry inside the backups directory could not be read.
    #[error("failed to read directory entry in {path}: {source}")]
    ReadDirEntry {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A pruned backup file could not be deleted.
    #[error("delete pruned backup {path}: {source}")]
    DeleteBackupFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    // ---- snapshot chain-DEK resolution ----
    /// A diff was about to be created but no base entry exists in the index.
    #[error("diff created without a base")]
    DiffWithoutBase,

    /// The base entry for the current chain does not have a wrapped DEK.
    #[error("base entry {id} missing wrapped DEK")]
    BaseEntryMissingWrappedDek { id: String },

    // ---- restore / pre-restore safety backup errors ----
    /// An encrypted entry in the chain has no passkey.
    #[error("backup entry {id} is encrypted but no passkey was provided")]
    EncryptedWithoutPasskey { id: String },

    /// The pre-restore safety-backup directory could not be created.
    #[error("failed to create pre_restore directory: {0}")]
    CreatePreRestoreDir(#[source] std::io::Error),

    /// The pre-restore safety copy (copy step) failed.
    #[error("stage pre-restore safety backup: {0}")]
    StagePreRestoreBackup(#[source] std::io::Error),

    /// The pre-restore safety copy (rename step) failed.
    #[error("commit pre-restore safety backup: {0}")]
    CommitPreRestoreBackup(#[source] std::io::Error),

    /// The target backup id was not found in the index during restore.
    #[error("unknown backup id: {id}")]
    RestoreUnknownId { id: String },

    /// A diff backup has no base_id field during restore.
    #[error("diff {id} has no base_id")]
    RestoreDiffNoBaseId { id: String },

    /// The chain root referenced during restore is missing from the index.
    #[error("chain root {id} missing")]
    RestoreChainRootMissing { id: String },

    /// The backup chain is archived under a different passkey.
    #[error(
        "backup chain {target_id} is archived under passkey fingerprint {fingerprint}. \
         Provide that passkey with --archived-passkey (or LIBLLM_ARCHIVED_PASSKEY) to restore."
    )]
    ArchivedChainKnown {
        target_id: String,
        fingerprint: String,
    },

    /// The backup chain has no recorded passkey fingerprint and is likely from a different passkey.
    #[error(
        "backup chain {target_id} has no recorded passkey fingerprint \
         (likely produced by `rebuild index` on a blob from a different passkey). \
         Provide the originating passkey with --archived-passkey to restore."
    )]
    ArchivedChainUnknown { target_id: String },

    /// The chain replay succeeded but the plaintext hash did not match the recorded value.
    #[error("hash mismatch after chain replay: expected {expected}, got {actual}")]
    RestoreHashMismatch { expected: String, actual: String },

    /// Restoring without passkey: the plaintext db write failed.
    #[error("failed to write restored database: {0}")]
    WriteRestoredDatabase(#[source] CryptoError),

    /// A temp file could not be created during encrypted restore.
    #[error("failed to create temp file for restore: {0}")]
    RestoreTempFile(#[source] std::io::Error),

    /// Writing plaintext to the temp file during encrypted restore failed.
    #[error("failed to write plaintext to temp file: {0}")]
    RestoreTempFileWrite(#[source] std::io::Error),

    /// Removing the existing database before encrypted restore failed.
    #[error("failed to remove existing database before encrypted restore: {0}")]
    RemoveExistingDatabase(#[source] std::io::Error),

    /// Opening the plaintext temp db for encrypted restore failed.
    #[error("failed to open plaintext temp db: {0}")]
    OpenPlaintextTempDb(#[source] rusqlite::Error),

    /// Exporting the plaintext database as encrypted failed.
    #[error("failed to export plaintext database as encrypted: {0}")]
    ExportAsEncrypted(#[source] rusqlite::Error),

    // ---- export errors ----
    /// The database could not be opened for export.
    #[error("failed to open database for export: {0}")]
    ExportOpenDatabase(#[source] rusqlite::Error),

    /// A statement on the database during export failed.
    #[error("failed to execute database statement during export: {0}")]
    ExportDatabaseStatement(#[source] rusqlite::Error),

    /// The backup API operation during export failed.
    #[error("failed to run backup API during export: {0}")]
    ExportBackupApi(#[source] rusqlite::Error),

    /// The temp file path is not valid UTF-8 (required for SQLCipher ATTACH).
    #[error("temp file path is not valid UTF-8")]
    ExportTempPathNotUtf8,

    /// Reading the exported database bytes failed.
    #[error("failed to read exported database bytes: {0}")]
    ExportReadBytes(#[source] std::io::Error),

    /// Creating the temp file for export failed.
    #[error("failed to create temp file for export: {0}")]
    ExportCreateTempFile(#[source] std::io::Error),

    // ---- compression / diff errors ----
    /// zstd decompressor initialization failed.
    #[error("failed to initialize zstd decoder: {0}")]
    DecompressInit(#[source] std::io::Error),

    /// Reading decompressed bytes failed.
    #[error("failed to decompress backup payload: {0}")]
    DecompressRead(#[source] std::io::Error),

    /// The decompressed payload exceeded the size cap.
    #[error("decompressed backup payload exceeds {cap} byte cap")]
    DecompressTooLarge { cap: u64 },

    /// bsdiff diff computation returned an I/O error.
    #[error("failed to compute binary diff: {0}")]
    DiffCompute(#[source] std::io::Error),

    /// bsdiff patch application returned an I/O error.
    #[error("failed to apply binary patch: {0}")]
    PatchApply(#[source] std::io::Error),

    /// zstd compression failed.
    #[error("failed to compress backup payload: {0}")]
    Compress(#[source] std::io::Error),

    // ---- hash errors ----
    /// A file could not be opened for hashing.
    #[error("failed to open file for hashing: {path}: {source}")]
    HashOpenFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Reading a file during hashing failed.
    #[error("failed to read file for hashing: {path}: {source}")]
    HashReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    // ---- crypto errors ----
    /// Argon2id key derivation for the backup key failed.
    #[error("key derivation failed: {0}")]
    KeyDerivation(argon2::Error),

    /// XChaCha20-Poly1305 encryption failed.
    #[error("encryption failed: {0}")]
    Encrypt(chacha20poly1305::Error),

    /// XChaCha20-Poly1305 decryption failed (authentication error).
    #[error("decryption failed: {0}")]
    Decrypt(chacha20poly1305::Error),

    /// The ciphertext blob is too short to contain a nonce and tag.
    #[error("ciphertext too short: expected at least {expected} bytes, got {actual}")]
    CiphertextTooShort { expected: usize, actual: usize },

    /// An unwrapped DEK has the wrong byte length.
    #[error("unwrapped DEK has wrong length: got {actual}, expected 32")]
    UnwrappedDekWrongLength { actual: usize },

    // ---- libllm::crypto forwarded errors ----
    /// A call into `libllm::crypto` (salt, key derivation, atomic write) failed.
    #[error("{0}")]
    LibllmCrypto(#[source] CryptoError),

    // ---- rekey errors ----
    /// The rekey journal could not be serialized.
    #[error("serialize rekey journal: {0}")]
    RekeyJournalSerialize(#[source] serde_json::Error),

    /// Atomic write of the rekey journal failed.
    #[error("atomic write of rekey journal: {0}")]
    RekeyJournalWrite(#[source] CryptoError),

    /// Reading the rekey journal file failed.
    #[error("read {path}: {source}")]
    RekeyJournalRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Parsing the rekey journal failed.
    #[error("parse rekey journal: {0}")]
    RekeyJournalParse(#[source] serde_json::Error),

    /// Removing a file during rekey cleanup/rollback failed.
    #[error("remove {path}: {source}")]
    RekeyRemoveFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Copying the index sidecar during rekey preparation failed.
    #[error("stage index.json.pre-rekey: {0}")]
    RekeyStageIndexSidecar(#[source] std::io::Error),

    /// Restoring the index from the sidecar during rollback failed.
    #[error("restore index.json from sidecar: {0}")]
    RekeyRestoreIndexSidecar(#[source] std::io::Error),

    /// An active base chain is missing its wrapped DEK during rekey.
    #[error("active chain {id} missing wrapped DEK")]
    RekeyMissingWrappedDek { id: String },

    /// Unwrapping a DEK during rekey failed.
    #[error("unwrap DEK of active chain {id}: {source}")]
    RekeyUnwrapDek {
        id: String,
        #[source]
        source: Box<BackupError>,
    },

    /// The rekey journal is present but the current passkey fingerprint matches neither old nor new.
    #[error(transparent)]
    RekeyJournalUnresolvable(Box<RekeyJournalUnresolvable>),

    /// No KEK was supplied but a rekey journal was found.
    #[error(
        "rekey journal found but no KEK supplied; \
         run with the passkey active at rekey time"
    )]
    RekeyJournalNoKek,

    // ---- orphan sidecar removal ----
    /// Removing an orphaned rekey sidecar failed.
    #[error("remove orphan sidecar {path}: {source}")]
    RemoveOrphanSidecar {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    // ---- migration errors ----
    /// Reading a backup file during migration failed.
    #[error("read {path}: {source}")]
    MigrationReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Decrypting a backup file during migration failed.
    #[error("decrypt {id} with current KEK: {source}")]
    MigrationDecrypt {
        id: String,
        #[source]
        source: Box<BackupError>,
    },

    /// Re-encrypting a backup file during migration failed.
    #[error("re-encrypt {id} under DEK: {source}")]
    MigrationReEncrypt {
        id: String,
        #[source]
        source: Box<BackupError>,
    },

    /// Staging (atomic write) a migrated file failed.
    #[error("stage {path}: {source}")]
    MigrationStageFile {
        path: PathBuf,
        #[source]
        source: CryptoError,
    },

    /// Renaming a staged migrated file to its final location failed.
    #[error("rename {src} -> {dst}: {source}")]
    MigrationRenameFile {
        src: PathBuf,
        dst: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The index could not be persisted during or after migration.
    #[error("persist index after chain {chain_id}: {source}")]
    MigrationPersistIndex {
        chain_id: String,
        #[source]
        source: Box<BackupError>,
    },

    /// Migration of encrypted index was attempted without a KEK.
    #[error(
        "cannot migrate encrypted backup index without a KEK: \
         re-run with a passkey set (LIBLLM_PASSKEY or --passkey)"
    )]
    MigrationEncryptedNoKek,

    /// Migration to v3 of encrypted index was attempted without a KEK.
    #[error(
        "cannot migrate encrypted backup index to v3 without a KEK: \
         re-run with a passkey set (LIBLLM_PASSKEY or --passkey)"
    )]
    MigrationV3EncryptedNoKek,

    /// A base entry is encrypted but has no wrapped DEK to embed at v3.
    #[error("base {id} is encrypted but has no wrapped DEK to embed at v3")]
    MigrationV3MissingWrappedDek { id: String },

    /// A v3 migration file rewrite failed.
    #[error("rewrite {path} with header: {source}")]
    MigrationV3RewriteFile {
        path: PathBuf,
        #[source]
        source: CryptoError,
    },

    /// Persisting the index after v3 migration reconciliation failed.
    #[error("persist index after reconciling metadata for {id}: {source}")]
    MigrationV3PersistReconciled {
        id: String,
        #[source]
        source: Box<BackupError>,
    },

    /// Persisting the index after embedding a v3 header failed.
    #[error("persist index after embedding header for {id}: {source}")]
    MigrationV3PersistEmbedded {
        id: String,
        #[source]
        source: Box<BackupError>,
    },

    /// An unknown migration version was requested.
    #[error("no migration registered for version {version}")]
    MigrationUnknownVersion { version: u32 },

    /// A v1->v2 migration step failed.
    #[error("v1 -> v2 migration failed: {0}")]
    MigrationV1ToV2(#[source] Box<BackupError>),

    /// A v2->v3 migration step failed.
    #[error("v2 -> v3 migration failed: {0}")]
    MigrationV2ToV3(#[source] Box<BackupError>),

    // ---- chain replay ----
    /// Reading the base backup file during chain replay failed.
    #[error("failed to read base backup: {filename}: {source}")]
    ReplayReadBase {
        filename: String,
        #[source]
        source: std::io::Error,
    },

    /// Reading a diff backup file during chain replay failed.
    #[error("failed to read diff backup: {filename}: {source}")]
    ReplayReadDiff {
        filename: String,
        #[source]
        source: std::io::Error,
    },
}

/// Detail payload for [`BackupError::RekeyJournalUnresolvable`]. Boxed inside the
/// enum so the common `BackupError` stays small (clippy `result_large_err`).
#[derive(Debug, thiserror::Error)]
#[error(
    "rekey journal present but current passkey fingerprint ({current}) matches neither \
     old_fp ({old_fp}) nor new_fp ({new_fp}); cannot auto-recover. \
     To abandon the rekey manually: (1) rename '{sidecar}' over '{index}' to restore the \
     pre-rekey index, (2) delete '{journal}' to remove the journal."
)]
pub struct RekeyJournalUnresolvable {
    pub current: String,
    pub old_fp: String,
    pub new_fp: String,
    pub sidecar: PathBuf,
    pub index: PathBuf,
    pub journal: PathBuf,
}

/// Convenience alias used throughout the backup crate.
pub type Result<T> = std::result::Result<T, BackupError>;
