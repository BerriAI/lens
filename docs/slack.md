# Lens findings in Slack

Lens can post new improvement candidates to one Slack channel. Each message contains a finding title, the number of supporting sampled runs, and a link to its evidence in Lens. Messages explicitly state that a paired eval has not established a quality or speed improvement. Trace excerpts, prompts, responses, and analysis descriptions stay in Lens

Create a Slack app named Lens, set its icon using the repository's Lens artwork, and enable Incoming Webhooks. Install a webhook into the intended channel following [Slack's incoming webhook guide](https://docs.slack.dev/messaging/sending-messages-using-incoming-webhooks/). The webhook inherits the app name, icon, and channel. This integration does not request access to Slack conversations

Set these environment variables on the Lens service and restart it:

```dotenv
LENS_SLACK_WEBHOOK_URL=<secret incoming webhook URL>
LENS_PUBLIC_URL=https://your-lens.example.com
LENS_SLACK_AGENT=moyai
LENS_SLACK_TEAM_ID=<Lens team ID>
```

The team ID is the Lens tenant ID, not a Slack workspace ID. Team-scoped delivery excludes investigations whose scope covers all teams. For a single organization's admin investigations, omit `LENS_SLACK_TEAM_ID` and explicitly set `LENS_SLACK_ALL_TEAMS=true`. This permits findings across all Lens tenants for the exact selected agent, so use a channel authorized for that scope. A team ID and all-team access cannot be enabled together

Keep the webhook URL in the deployment's secret settings. Only official HTTPS Slack and GovSlack webhook hosts are accepted, and redirects are disabled. Finding titles are sent as plain text so captured markup cannot mention users or channels. Do not include sensitive information in finding titles destined for Slack

Analysis must already be configured and receiving production traces. Notifications do not start model calls, change sampling, or run agent tasks. The first successful scan records an activation time and skips existing findings. Subsequent scans run once per minute and deliver only newly created, open issue findings with supporting evidence for the configured agent and scope. Resolved findings, dismissed findings, positive patterns, and changes to already delivered findings do not generate messages

Delivery state is saved in Lens's existing ClickHouse state store and survives restarts. Each finding gets at most six attempts with backoff for transport failures, rate limits, and server errors. Permanent HTTP errors stop retries. At most five attempts run per scan, separated by one second. Failed delivery emits a generic service warning and never fails an investigation or eval

A crash or lost response after Slack accepts a message can produce a duplicate. Webhooks do not return a message identifier that this integration can reconcile. Exhausted attempts are not retried automatically. Rotating the webhook or changing the selected scope creates a new activation time and skips older findings. Findings removed, merged away, or resolved before a successful scan may never be sent. Review Findings in Lens for the authoritative history

## Proving an improvement

This first integration reports investigation candidates only. To establish a measured improvement, convert relevant findings into a reviewed dataset, add outcome assertions or a calibrated judge, and run baseline and candidate code against the same frozen cases and scorer configuration. Keep a separate held-out set for confirmation after choosing a candidate

Lens already supports [PR evals](github-evals.md) and [Moyai's asynchronous task API](agent-io.md#moyais-asynchronous-task-api). The `task_completed` trace scorer checks root-span error status; it does not establish semantic task success. Paired comparisons should report commits, dataset revision, case counts, trial counts, changed cases, uncertainty, and regression guardrails. A latency or cost claim also needs complete comparable measurements. Findings alone are not evidence that a proposed fix works

Publishing paired eval results to Slack, creating fixes, interactive commands, and automated promotion are follow-up work
