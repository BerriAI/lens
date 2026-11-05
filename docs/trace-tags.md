# Filter investigations with LiteLLM tags

In Traces, click the search box on the left, choose `tag`, and select a value recorded by LiteLLM. You can also type an exact filter such as `tag:skill:diagram` or `tag:"User-Agent: codex"`. Multiple selections require all tags to match. The filter applies before pagination. Investigate tags opens setup with the same tags, agent and lookback; preview and sampling use the same conditions

LiteLLM request tags are strings, not key/value metadata. Lens preserves values such as `skill:diagram` and `User-Agent: codex` without splitting them. The search suggestions show up to 64 tags from accessible spend logs in the selected time range. It can include tags used by other accessible agents; the trace results still respect the selected agent

Lens reads canonical `spend_logs.request_tags` and correlates them to traces through recorded call identifiers and team identity, requiring either the same nonempty user or the same nonempty API-key identity, as in Lens cost attribution. Ambiguous identifiers do not attach tags. Structured JSON arrays in supported trace tag attributes are also recognized. Upstream tags are read-only in Lens. Additional Lens key/value tags remain separate and can be edited on the run

The current LiteLLM OTEL callback does not export all canonical request tags. A trace-only import may therefore lack tags that exist in LiteLLM. For those tags, Lens needs the corresponding spend logs and recorded request identifiers. Lens does not evaluate Python string representations or guess tags from prompt text

Recorded tag display and per-run request-tag aggregation are bounded to 64 entries. Filters accept at most eight conditions, with keys and values up to 256 bytes. The tag catalog loads while completing the `tag:` search field and is cached for one minute. Empty matches keep the filters visible so they can be removed or changed

Tag values use exact matching; wildcards and negation remain available for the other search fields. Shared links with existing tag filters still work and show removable chips next to the search. Copy as curl is hidden when tag filters are active, because the generic SQL copy does not include the dedicated tag lookup
