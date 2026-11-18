# Gateway cost ledger and offloaded bodies

LiteLLM can deliver each accepted spend record to Lens as a versioned ledger event instead of through the legacy spend callback. Lens stores one compact cost row per delivery, so retries and replays count once and separate charges for the same call each count once. Large request and response bodies can optionally move to an S3-compatible bucket that you operate, while Lens keeps a marked preview in its own tables

Both parts are off by default. Without them, Lens and LiteLLM behave as described in [Recorded gateway costs](../deploy/lens/README.md#recorded-gateway-costs)

## Enable the ledger

On the gateway, set `general_settings.lens_ledger_export: true`. The gateway also needs its Postgres database and the same `LITELLM_LENS_URL` and `LITELLM_LENS_SERVICE_TOKEN` it uses for the legacy export. Lens advertises ledger support on `/internal/status`, and the gateway switches a request to the ledger only when Lens reports version 1. Each request goes through exactly one producer, so a call is never exported by both paths

Lens accepts deliveries on `/internal/ledger` and body uploads on `/internal/ledger/payloads`. Both require the gateway service token. Nothing else can write to them, and users and tracing keys cannot reach them

Each request to Lens is limited to 32 MiB and 1,000 events. Lens rejects an event without a positive start time or a request id, and an event id outside `[A-Za-z0-9_-]{1,128}`, without blocking the other events in the batch

## Store large bodies in a bucket

Without a bucket, Lens keeps `messages` and `response` inline up to `LENS_PAYLOAD_INLINE_BYTES` (default 1 MiB per field). A longer value is cut to `LENS_PAYLOAD_PREVIEW_BYTES` (default 1 MiB) on a UTF-8 boundary, and the cut is marked with `…[Lens preview: N bytes omitted]`. JSON stays valid JSON

To keep full bodies, set these on Lens:

| Setting | Meaning |
| --- | --- |
| `LENS_PAYLOAD_S3_BUCKET` | Bucket name. Setting it turns the store on |
| `LENS_PAYLOAD_S3_PREFIX` | Optional prefix, empty or ending in `/` |
| `LENS_PAYLOAD_S3_ENDPOINT` | Endpoint URL for an S3-compatible service other than AWS |
| `LENS_PAYLOAD_S3_REGION` | Region, when it does not come from the AWS environment |
| `LENS_PAYLOAD_S3_PATH_STYLE` | `true` for services that need path-style addressing |

With a bucket the defaults become 64 KiB inline and a 4 KiB preview. Both settings must be at least 256 bytes, and the preview cannot be larger than the inline limit. Credentials come from the standard AWS chain (environment, shared profile, web identity or instance metadata). Lens never accepts bucket credentials from clients and never hands out presigned URLs

Lens writes objects only under `{prefix}lens-payloads/v1/YYYY/MM/DD/{scope}/{event_id}/{field}`, where the scope is a hash of the team and key. Grant its credentials `s3:PutObject`, `s3:GetObject` and `s3:DeleteObject` on that namespace and `s3:ListBucket` limited to that prefix. Lens does not need access to anything else in the bucket and never deletes outside the namespace

The store must support ListObjectsV2 in key order, DeleteObjects, and SHA-256 object checksums. AWS S3 general purpose buckets meet this. S3 directory buckets do not list in key order, so expired objects there may not all be found. For other S3-compatible services, confirm these operations before you enable the store

## What readers see

For gateway calls recorded through the ledger, Lens fetches the full body after it has checked that the reader may see the row, and only from the key for that row's own team, key, event and field. The body is used only when its length and checksum match what was uploaded. This applies to span details in the trace view and to investigation reads, search and evidence checks

When the full body is not available, readers see the stored preview and the reason. Reasons are that the body has not been stored yet, that it expired or was deleted, or that it could not be read. Span details show the reason in the `lens.payload.input` or `lens.payload.output` attribute. Investigations get the reason after the preview and treat the execution as partial

Ad hoc SQL over `spend_logs` (the query API and the analysis sandbox) only ever sees the marked preview of an offloaded body

Spans that agents send directly over OTLP are unchanged. Their content is stored on the span and is never moved to the bucket

## Costs that arrive late

The gateway retries deliveries until Lens acknowledges them, so a cost can arrive minutes or hours after the call. A stored trace summary is recomputed when a delivery for any call it already priced or tried to price arrives, including a later charge for the same call. An open trace view refreshes within 5 seconds while it still has an unpriced call. Once every call is priced, it can take up to 10 minutes for the view to show a further charge

Late bodies need no refresh. Span details and investigation reads fetch the body when they are opened

## Retention

Ledger costs, aliases and body references use `AGENT_TRACING_RETENTION_DAYS`, like the other Lens tables. Lens sweeps the bucket when it starts and every six hours after that. It deletes generated objects dated before the retention window, starting from the oldest day and stopping at the first day it keeps, so a sweep after downtime still removes everything that expired in the meantime. Objects the store refuses to delete are logged and retried on the next sweep. Keys under the namespace that Lens did not generate are left alone

## Turn it off

To return to the legacy export, set `lens_ledger_export` back to `false` on the gateway. Deliveries already queued keep draining while Lens is reachable, and ledger costs already stored stay readable. Lens hides a legacy row when a ledger delivery for the same team, key and call exists, so a call exported both ways counts once

If you remove the bucket settings from Lens, it stops uploading and sweeping. Bodies already offloaded show their marked preview, and the objects stay in the bucket. Delete the `lens-payloads/` namespace yourself, or add a lifecycle rule for it, when you no longer need them
