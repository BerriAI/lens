# Back up and restore Lens

The Compose installation provides a cold backup of Lens's complete ClickHouse volume, including coordination state, deployment credentials and the exact Lens image. The backup briefly stops ingestion and background work. Schedule it when your exporters can buffer or retry, and keep the resulting directory private: it contains credentials and trace content.

This procedure applies to the bundled single-node ClickHouse installation from [Run Lens](../deploy/lens/README.md). A supported external single-server ClickHouse installation needs a backup procedure covering its data and Keeper state together; the Compose helper does not manage it. Multi-server ClickHouse storage is outside the [supported topology](../helm/lens/README.md#use-existing-clickhouse)

## Create a backup

From your Lens checkout or unpacked installation bundle:

```sh
./deploy/lens/backup "$HOME/lens-backup-$(date +%Y%m%d-%H%M%S)"
docker compose -f deploy/lens/compose.yaml ps
```

The destination must be a new directory, and the disk must have space for the database and Lens image archive. If you use a custom Compose project name, supply the same `COMPOSE_PROJECT_NAME` used to start that deployment.

The helper stops Lens and ClickHouse, copies the stopped volume, saves the environment file, Compose and Keeper configuration, archives the installed Lens image, and records SHA-256 checksums. It then restarts the services that were running before the backup, including when copying fails. Wait for both services to become healthy and open a recent trace. An incomplete backup has no valid complete checksum manifest and cannot be restored by the helper.

Move the entire backup directory to your approved protected storage. Keep the matching installation bundle or source revision as well. Choose a backup cadence and retention period based on how much recent data you can afford to lose.

## Restore into a new deployment

Use the same installation bundle or source revision as the backup. The restore helper verifies that its Compose and Keeper files match the saved files. Use a fresh directory without `deploy/lens/.env` and a new Compose project name; it refuses to overwrite an existing environment, container or volume.

For a recovery rehearsal alongside your existing local installation, choose an unused port:

```sh
COMPOSE_PROJECT_NAME=lens-restored \
LENS_PORT=4320 \
LENS_PUBLIC_URL=http://localhost:4320 \
./deploy/lens/restore "$HOME/lens-backup-20261008-210000"
```

Replace the backup path with your saved directory. The helper verifies every backup file, loads the archived Lens image, restores ClickHouse and Keeper into an empty volume, and starts both services. It retains the saved credentials and records the new project name and exact image identifier in the new environment file. The port and URL overrides above apply to this command; save them in the restored `deploy/lens/.env` before subsequent starts if this deployment will keep that address.

Open [the restored Lens](http://localhost:4320/ui/) and use the administrator token from the restored environment file. Confirm that a known trace opens, a dataset has its expected revision and cases, and completed investigations and evaluations are present when your backup includes them. A healthy service alone does not prove recovery.

For production recovery, use your deployment's public URL and TLS proxy. Keep writers pointed at one deployment during cutover. Exporters can resume using retained tracing keys after you have verified the restored data and routed them to it. Data received after the backup is outside that recovery point.

## Reclaim old state versions

Lens retains the current value of each state record and a revision fence that prevents stale workers from publishing old work. Compaction removes superseded or abandoned payload versions and their derived indexes. It preserves current investigations, datasets, sessions, keys and settings. It does not delete historical product records or change trace retention.

For the bundled Compose deployment, take a backup first, then stop Lens while ClickHouse remains running:

```sh
docker compose -f deploy/lens/compose.yaml stop lens
docker compose -f deploy/lens/compose.yaml run --rm --no-deps lens compact-state
docker compose -f deploy/lens/compose.yaml up -d --wait
```

The command reports `Compacted N state records; current values and revision fences retained`. Repeating it is safe. If it fails, restart Lens with the last command, inspect the error and retry after resolving it. A partial run leaves current data readable; it may leave some old versions to reclaim on the next run.

Check a known dataset export and completed investigation after maintenance. Compaction bounds the number of stored payload versions per existing record; storage still grows as you create new records. Include both ClickHouse data and Keeper memory in capacity monitoring.

## Diagnose a failed backup or restore

| Result | Action |
| --- | --- |
| Backup destination exists | Choose a new directory; the helper does not replace previous backups |
| Copy or image archive fails | Check available disk space and Docker logs; confirm the original services resumed before retrying |
| Backup verification fails | Restore a complete unchanged backup, including hidden `.env`; do not bypass the checksum check |
| Compose or Keeper files differ | Use the installation files from the backup's version |
| Target environment, containers or volumes exist | Use a fresh directory and project name; preserve the existing deployment |
| Restored service is unhealthy | Inspect `docker compose -f deploy/lens/compose.yaml logs --tail=100` from the restored directory and check available disk and memory |

The automated recovery check creates a trace and dataset revision, backs up and restores a separate installation, compares the dataset and export, and verifies the retained session and tracing key. It also checks refusal to overwrite existing deployments and rejection of damaged backups:

```sh
node deploy/lens/recovery-smoke.mjs lens:local
```

This check requires Node.js and Docker and uses temporary test deployments. Replace `lens:local` with the exact image you intend to qualify. It does not modify your existing deployment.
