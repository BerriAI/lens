# Large request content

LiteLLM sends one spend record per request to Lens. A record whose prompt and response fit in 1 MiB carries them inline, exactly as before. A larger record used to be dropped whole, cost included. Now LiteLLM keeps the cost and metadata and either stores the content in S3 or marks it unavailable

## Setup

On LiteLLM, configure the `s3_v2` logging callback with its bucket and credentials, then opt in under the tracing store:

```yaml
general_settings:
  tracing:
    store:
      type: lens
      content_storage: s3_v2
      content_inline_bytes: 1048576
      content_max_bytes: 8388608
```

`content_inline_bytes` is the largest record sent inline, and `content_max_bytes` is the largest stored object. Both settings are optional and capped at those defaults. Without `content_storage`, oversized records keep their cost and are marked `storage_not_configured`

On Lens, set `LENS_GATEWAY_API_BASE` and `LENS_GATEWAY_API_KEY` to the LiteLLM gateway, plus the same `LITELLM_LENS_SERVICE_TOKEN` that LiteLLM uses. Lens reads stored content from the gateway's internal `POST /lens/content` route with that token. Lens never holds S3 credentials

## What readers see

Span details, investigation content and evidence checks load stored content on demand. A signed reference in the record's `litellm_lens_content` metadata key ties each object to its team, key, request and SHA-256 digest. Lens and the gateway both reject a reference that does not match its row, and they reject an object whose size or digest changed. When content cannot be loaded, span details report `litellm.content.state` and `litellm.content.reason` attributes, and investigation text says `[content unavailable: <reason>]`. That placeholder never counts as evidence

List previews and ad hoc SQL still see the record's empty content columns and the reference metadata

## Limits

The S3 object is the only copy of oversized content. Its lifetime follows the bucket's lifecycle rules, not Lens retention. References are signed with a key derived from `LITELLM_LENS_SERVICE_TOKEN`. Rotating that token makes every earlier reference unreadable, and those records then show as unavailable. Uploads and reads are bounded in count, bytes and time. Content that is in flight when LiteLLM shuts down is recorded as `cancelled`, and it is not retried later
