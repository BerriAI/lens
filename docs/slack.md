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

Publishing verified paired eval results to Slack, creating fixes, and automated promotion are follow-up work

## Ask Lens in a thread

The optional Node 24.20.0 sidecar uses the official [OpenAI Agents SDK](https://developers.openai.com/api/docs/guides/agents/sdk) and [Slack Bolt Socket Mode](https://docs.slack.dev/tools/bolt-js/concepts/socket-mode/). Add the bot scopes `app_mentions:read`, `chat:write` and `files:write`, subscribe to `app_mention`, enable Socket Mode, and create an app token with `connections:write`. Reinstall the Slack app after changing scopes, then invite Lens to the configured channel

```dotenv
LENS_SLACK_CHAT_ENABLED=true
LENS_SLACK_BOT_TOKEN=<secret xoxb token>
LENS_SLACK_APP_TOKEN=<secret xapp token>
LENS_SLACK_WORKSPACE_ID=<Slack workspace ID>
LENS_SLACK_CHANNEL_ID=<Slack channel ID>
LENS_SLACK_AGENT=<exact agent name>
LENS_SLACK_ALL_TEAMS=true
LENS_PUBLIC_URL=https://your-lens.example.com
OPENAI_API_KEY=<secret model provider key>
OPENAI_BASE_URL=https://api.openai.com/v1
LENS_SLACK_MODEL=gpt-6-luna
```

The model and base URL can point to an OpenAI-compatible gateway; use the model alias configured on that gateway. Lens reads use `LENS_SLACK_LENS_API_KEY` when set, otherwise the existing `LENS_ADMIN_TOKEN`. The internal address comes from the server's `LITELLM_LENS_LISTEN` setting, including its port, with wildcard listeners mapped to loopback. Optional `LENS_SLACK_SERVICE` defaults to the selected agent; trace scope includes that exact service or exact agent membership

Mention Lens to ask about recent runs, open findings, or eval counts. Replies stay in the source thread. Ordinary mention conversations retain the last two exchanges for up to an hour, with at most 100 threads in memory

Accepted requests display Slack's native "Lens is working…" indicator in the reply thread. The indicator refreshes every minute and clears when the response succeeds or fails. Status failures do not prevent the answer. This follows [AgentChat's working-status lifecycle](https://github.com/BerriAI/agentchat/blob/main/src/agentchat/activity.py) and uses the existing `chat:write` scope through `assistant.threads.setStatus`. Slack's [March 2026 scope update](https://docs.slack.dev/changelog/2026/03/05/set-status-scope-update/) allows channel apps to use this loading state without requesting `assistant:write`

The green availability dot is a separate app setting, `features.bot_user.always_online`. It indicates that the bot is available, not whether a request is running or the worker is healthy. The Agents setting and agent-session UI are also separate; enabling them adds `assistant:write`. The thread indicator does not require enabling that setting or adding DM access

To chat with a finding without mentioning Lens again, grant `groups:history`, subscribe to `message.groups`, reinstall the app, then set `LENS_SLACK_THREAD_REPLIES_ENABLED=true`. This private-channel scope lets Slack deliver messages from private conversations Lens joins; the worker accepts only the configured workspace and channel, and unmentioned replies only in registered Lens finding threads. It does not fetch channel history, join channels, or answer unrelated conversations. The message-identity deduplication and subscribed-thread routing follow [AgentChat's Slack adapter](https://github.com/BerriAI/agentchat/blob/main/src/agentchat/channels/slack.py)

Each posted finding registers its Slack root timestamp, issue number and exact trace references in ClickHouse. A follow-up reloads those traces and their root input/output before invoking the model, then allows bounded reads of observed spans in those traces only. It answers the follow-up directly instead of substituting a recent-runs report. The last three exchanges and twenty handled message timestamps persist across restart. A message delivered both as `message` and `app_mention` receives one answer. Accepted messages are claimed before invoking the model; a crash can lose that answer rather than blindly duplicating it. Trace content remains untrusted evidence and cannot issue instructions or expand tool access

For visible acknowledgement, separately grant `reactions:write` and set `LENS_SLACK_REACTIONS_ENABLED=true`. Lens adds eyes while reading, replaces it with a check when its reply is delivered, or an x on failure. Reaction errors never prevent the answer. Existing findings can be registered with the operator-only `LENS_SLACK_FINDING_THREADS` JSON array of `{thread, finding}` records matching `threadSchema`; at most ten seeds are accepted, and each trace must match the configured agent, Lens origin, trace ID and trace reference. This bootstrap cannot be supplied through Slack or model output

Reports use native Slack blocks with Performance, Agent quality and Reliability categories, at most three supported opportunities, verified numbered trace links, and assessed impact out of ten. A required PNG chart appears in the same first report message and uses `files:write`; add that bot scope and reinstall the app before enabling reports. The PNG renders before posting, then uploads into the configured channel's report thread to grant members access. Lens updates its original report with the chart; no extra top-level alert or internet-public file URL is created. Upload or update failure is reported as incomplete delivery and can leave a partial card. Ordinary errors and non-report replies do not fabricate chart data. Count labels state the measured cohort, and an empty category has no supported finding. The bundled ABeeZee font is distributed under its included SIL Open Font License

Chat currently requires explicit all-team access because the existing trace summary endpoint omits tenant identifiers, so a configured team cannot be independently verified using an admin credential. It rejects `LENS_SLACK_TEAM_ID`; use a channel authorized to see the configured agent across all Lens tenants. The existing webhook worker still supports its narrower team scope

Each process allows one chat answer at a time, 20 answers per hour and 100 per day. Each answer is limited to eight model turns, 1,200 output tokens per turn, bounded tool results and a 90-second deadline. General reports read each evidence tool at most once. Finding follow-ups reload up to four exact trace references and permit at most eight additional observed-span reads. These are request limits, not a financial guarantee. SDK trace export and response storage are disabled. Ordinary mention history and chat budgets reset on restart; registered finding history and message claims persist. Events older than five minutes are ignored. Run one service instance for these chat limits to apply globally

The container starts the sidecar only when chat is enabled and the server runs normally; existing server CLI commands retain their arguments and exit status. Sidecar failures leave Lens running and get at most three delayed restarts. Investigator configuration failures leave mention chat available

## Scheduled production investigations

The same sidecar can check for new production evidence every ten minutes. Enable it only after configuring chat, the existing Lens GitHub App, and the existing ClickHouse HTTP connection

```dotenv
LENS_INVESTIGATOR_ENABLED=true
LENS_INVESTIGATOR_MODEL=gpt-6-astra
LENS_INVESTIGATOR_REPOSITORY=your-org/your-agent
LENS_INVESTIGATOR_DAILY_RUN_LIMIT=144
```

Use the appropriate gateway model alias when applicable. `CLICKHOUSE_URL` (or `CLICKHOUSE_HOST`, `CLICKHOUSE_USER`, `CLICKHOUSE_PASSWORD`), `CLICKHOUSE_DATABASE`, `LENS_GITHUB_CLIENT_ID` and `LENS_GITHUB_PRIVATE_KEY` reuse the server settings. The GitHub App must have contents access to the explicitly configured repository. Each investigation mints a short-lived installation token restricted to that repository with contents-read permission, resolves its default branch to a commit SHA, and reads source files at that fixed SHA

The worker scans retained trace metadata from the beginning of history, up to 100 pages, and skips unchanged samples. It preloads up to 100 trace details with 500 spans each, each exact interactive root's bounded input/output, and first/last model context for partial traces. Larger or truncated histories are explicitly incomplete. Root turns, delegated children, model-only fragments and the exact Lens setup check have separate classifications; a missing parent span never establishes completion. Timing totals are unavailable when a detail page is truncated. Astra can additionally read eight observed spans and six source files, within twenty model turns plus one final synthesis call and a four-minute deadline. It does not execute code, patch files, start evals or deploy changes. Repository code is current default-branch code, not confirmed deployed code

Only candidates with an assessment of at least 0.85 confidence and impact at least 5 out of 10 qualify. Confidence is uncalibrated model judgment, not a statistical probability. Impact 5–6 means blocked or repeated degraded tasks, 7–8 means frequent substantial failures, and 9–10 means critical widespread failure. Ordinary issues require two verified human root traces with distinct known incident identities. A feature request can qualify with one exact current-user quote and a readable request denominator; severe frustration additionally requires explicit frustrated user language. Historical Slack reference sections and attachments do not count as current requests. Literal quote support is labeled observed request support, not demand prevalence. Every quoted span excerpt and code location must match material the tools actually read. Regression candidates additionally require identical observed complete root input and opposing terminal error outcomes or at least doubled observed duration. Invalid comparisons are downgraded to candidates, never claimed as controlled benchmarks

A qualifying report has a durable Lens issue number, a purple Proposed status border, Bug Fix or Feature heading and category, assessed impact, verified affected-user identifiers with numbered trace links, a host-computed frequency, conditional customer benefit, and a required chart in its first message. Numeric cohorts require their exact measured title and cited membership; unknown frequencies stay quiet. The thread starts with User tried → Agent behavior → Observed failure or gap receipts, then code references, observed metrics, limitations and the required experiment. These issue numbers belong to this scoped investigator state, not GitHub issues or the separate native Findings UI. Captured content is untrusted, obvious credential patterns are redacted, and text is escaped to prevent mentions. Short trace excerpts, exact verified user IDs and code hypotheses are sent to the configured provider and Slack channel; use a channel authorized for that data

ClickHouse saves the check cadence, ownership claim, daily budgets, sample signature, issue numbers, candidate fingerprints and provenance. Claims use existing KeeperMap revision checks, so overlapping workers do not independently reserve the same work. At most one candidate is attempted per cycle and three per UTC day. The default allows up to 144 paid investigations per UTC day, each with up to 21 model calls; lower the daily limit to control provider usage. Ten-minute checks continue when the inference budget or posting budget is exhausted, but do not run Astra. Empty samples, unchanged samples and quiet investigations produce no Slack message. Provider failures consume the reserved inference budget but do not mark that evidence successfully analyzed

Delivery is recorded before the Slack attempt to prevent blind duplicate retries. A definitive Slack rejection naming a newly uploaded invalid image is retried after 1, 2, 4 and 8 seconds because private image readiness can lag upload completion. No network or ambiguous delivery error is retried. A crash, transport error or partial thread delivery can therefore lose a report. Failed attempts consume the posting budget. Deduplication retains the latest 200 attempted candidate fingerprints; semantically identical issues described differently may still recur. Changing the repository, workspace, channel or selected agent creates a new state identity. State failures fail closed and never authorize a report or additional model call

The worker stores model, prompt revision, selected trace IDs, repository SHA and time with its latest completed investigation and attempted reports. It does not claim a percentage improvement. Proving an improvement still requires a reviewed code change and an executed baseline/candidate benchmark on frozen cases, with held-out confirmation and quality, latency and cost guardrails

## Development checks

Previously published manual reports can seed an empty investigator state using `LENS_INVESTIGATOR_KNOWN_FINDINGS`, with an exact `[workspace, channel, agent, repository]` scope and validated `stateSchema.sent` records. Bootstrap never overwrites existing state. Matching measured finding titles suppress a duplicate report even if the model chooses a different issue key

The shared Findings agent lives in `src/agent/agent.ts`. Its official SDK prompts, tools and execution serve both conversational analysis and scheduled investigations. Domain verification, Lens reads, repository access and durable scheduling remain in that package. It imports no Slack SDK and receives only model and Lens configuration. The Slack connector lives in `src/agent-chat/connectors/slack`; it calls the core, validates the returned grounding metadata, and handles channel routing, formatting, uploads, reactions and working status. Native web Findings persistence is a separate integration from this module boundary.

Run `npm ci --ignore-scripts`, `npm run check` and `npm test` in `src/agent` first, then the same commands in `src/agent-chat`. The connector package links the local core package and requires its compiled declarations. Both suites run in the `lens-agent` workflow. Tests exercise the official SDK tool loop with a scripted model, Slack routing and limits, scoped evidence reads, durable-worker decisions, evidence verification and process supervision. They do not send Slack messages or call a paid model
