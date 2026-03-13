# MuonCache persistence + shutdown spec

Date: 2026-02-01

## Goals
- Provide a release-mode workflow for running muoncache with persistence enabled.
- Ensure graceful shutdown on Ctrl+C (SIGINT) or scripted stop, flushing data to the persistence store.
- Provide a clear, repeatable way to start with an existing persisted file to restore data.
- Log the persisted file path and a DB snapshot summary before shutdown to prevent silent data loss.

## Behavior
- **Default port**: 6379 unless overridden.
- **Persistence enablement**: `--persist <path>` enables snapshot persistence (and loads from it on startup).
- **Graceful shutdown**:
  - On Ctrl+C (SIGINT), the server prints the DB contents summary and performs a snapshot.
  - After snapshot completes, the server exits cleanly with a confirmation message.
- **Stop command**:
  - A make target sends SIGINT to the background muoncache process (by PID), waits briefly, then force-kills if needed.
  - The SIGINT triggers the graceful shutdown sequence, ensuring snapshot is flushed.

## Make targets
- `muoncache-persist-release` starts a release server with persistence at a timestamped file in tmp/.
- `muoncache-persist-release-bg` starts the same in background and writes pid/port/db metadata to tmp/.
- `muoncache-stop` stops the background server via SIGINT.

## Data safety
- Snapshot occurs before exit.
- Snapshot completion is logged, including the target database path.
- The DB summary includes key names and types per DB.

## Load semantics
- `--persist <path>` on startup loads existing snapshot data for all DBs.
- The persisted file path is logged on startup and shutdown.
