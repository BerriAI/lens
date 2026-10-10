# Slack acknowledgment timing replay

This replay compares the actual `Chat` implementation at commit `55d6a2365a25828d2b9e21ff49ed3c6435e2fcd4` with the candidate implementation. Both versions receive identical delayed fixtures: 120 ms for working status, 180 ms for evidence and model response, and 10 ms per Slack write. It runs five paired trials without a provider or Slack connection

| Median across five trials | Before | After |
| --- | ---: | ---: |
| First visible text | 313.4 ms | 11.2 ms |
| Request completion | 313.4 ms | 203.8 ms |
| Slack writes per request | 1 | 2 |
| Visible reply messages | 1 | 1 |

The first visible text arrives 96.4% sooner in this controlled fixture. Before, the first text is the final answer after working status and evidence complete. After, it is an acknowledgment before evidence begins; the result updates that message. Completion also avoids waiting serially for working status, but still includes the extra acknowledgment write

These measurements demonstrate the scheduling change, not production latency or model quality. They do not measure Slack rendering, provider response time, task success or production trace processing. The current configured provider route rejected the attempted live analysis with an exhausted-credit error, so no live model improvement is claimed. Local Node was 24.19.0; deployment and CI pin 24.20.0

The [raw receipt](slack-acknowledgment.json) records all trials, fixture delays and exact Git source identities. Reproduce after installing and building the core package, then run this from `src/agent-chat`:

```sh
npm ci --ignore-scripts
npm run benchmark:ack
```

The replay loads the historical source through Git, so the baseline commit must be present locally. Absolute timings vary with host load. Separate regression tests cover a model that never settles, safe provider-quota failures, shutdown, delayed activity, bounded context failures and a chart upload that finishes after timeout. No test calls a paid model or posts a Slack message
