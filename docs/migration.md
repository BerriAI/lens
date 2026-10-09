# Move existing Lens metadata to ClickHouse

Use `lens-migrate` when your Lens investigations, workers, tracing keys, datasets
or signals still live in LiteLLM's PostgreSQL database. It is a separate Rust
executable included in the Lens image. PostgreSQL is needed only while reading
the old installation; the running Lens service uses ClickHouse

If Lens metadata already lives in ClickHouse, keep that database and its Keeper
volume. Do not run the PostgreSQL importer over it. Traces, spend logs and
feedback already stored in ClickHouse remain in place in either case

## Prepare and inspect

Back up the PostgreSQL database and the ClickHouse volume before cutover. Stop
the old Lens writers, including its workers and signal scheduler. Keep the new
Lens service stopped until migration verification finishes. A repeatable-read
snapshot makes the import consistent, but changes written after that snapshot
are not copied

Use a PostgreSQL account with SELECT permission on the eight `LiteLLM_Lens*`
tables. Set `LENS_MIGRATION_POSTGRES_URL` in your shell through your usual secret
manager. The target ClickHouse database may contain existing traces and feedback,
but must have no Lens state records from another installation

For the repository's Compose deployment, start ClickHouse on its own and inspect
the migration plan with the image you intend to deploy:

```bash
docker compose --project-directory deploy/lens -f deploy/lens/compose.yaml stop lens
docker compose --project-directory deploy/lens -f deploy/lens/compose.yaml up -d --wait clickhouse
docker compose --project-directory deploy/lens -f deploy/lens/compose.yaml \
  run --rm --no-deps -e LENS_MIGRATION_POSTGRES_URL \
  --entrypoint /usr/local/bin/lens-migrate lens
```

The command prints source row counts and a snapshot fingerprint. It makes no
changes to PostgreSQL or ClickHouse. Investigations include current findings and
jobs; archived jobs, review checkpoints, dataset revisions, worker token hashes,
tracing key scopes/expiry and signal state are included. The tool retains stored
identities and scope values and rejects unsupported or inconsistent records

The importer reads Lens metadata into memory for validation before writing.
Give this one-time process enough memory for the metadata snapshot. Trace bodies
and spend logs are not loaded from PostgreSQL

## Import and verify

After checking the plan, apply that snapshot with both Lens runtimes still stopped:

```bash
docker compose --project-directory deploy/lens -f deploy/lens/compose.yaml \
  run --rm --no-deps -e LENS_MIGRATION_POSTGRES_URL \
  --entrypoint /usr/local/bin/lens-migrate lens --apply --source-stopped
```

The importer creates the ClickHouse state tables and indexes before inserting
records. It reads each imported value back and reports `applied: true` and
`verified: true` only when verification completes. A successful import retains
the same Lens IDs, dataset revisions, credential hashes and archived job links

Start Lens with the same ClickHouse database. Confirm that the expected team can
see its investigations and datasets, tracing keys still ingest under the correct
scope, and the stored findings and archived runs are visible. Keep PostgreSQL
available until that check is complete

The Compose command inherits the deployment's ClickHouse settings. For another
deployment, override the image entrypoint with `/usr/local/bin/lens-migrate` and
pass `LENS_MIGRATION_POSTGRES_URL`, `CLICKHOUSE_URL` or the existing
`CLICKHOUSE_HOST`/`CLICKHOUSE_USER`/`CLICKHOUSE_PASSWORD` settings, and
`CLICKHOUSE_DATABASE`. Use a network that can reach both databases. No database
credentials are printed in the plan

## Interrupted import and rollback

If the import stops, keep both runtimes stopped and run the same apply command
again. A checkpoint binds the target to the source snapshot fingerprint. Matching
records are verified and retained; missing records are copied. Different source
data, changed target values or unrelated Lens state stop the import instead of
overwriting records

Do not change the source after an interrupted import or start Lens before
verification succeeds. To abandon that attempt, restore the target from its
pre-migration backup or select a fresh target database. The importer never
deletes source data. Before new writes begin, rolling back means restarting the
old deployment against its unchanged PostgreSQL database. After new writes begin,
preserve and reconcile those changes before switching back
