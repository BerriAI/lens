# Continuous conversation context

The selected run previously started at its current request even when earlier turns were already recorded under the same session. The Thread view now shows the recorded requests and replies as one continuous conversation, with links to inspect their original runs

The screenshots use built-in demo data. They contain no production conversation data

## Reproduce

1. Start the UI with `npm run dev:ui` and open `http://localhost:3100/ui/?tab=traces&demo=true`
2. Select `support_agent` and open the run whose input is `Where is order #1042?`
3. Click **View full conversation**, then **Enter full screen**
4. Confirm that `Can you help me track my order?`, the assistant reply and `Where is order #1042?` appear in chronological order without section dividers
5. Click **Inspect run** and confirm that the source trace opens separately

## Before

![The selected run starts with its current request](before.jpg)

## After

![Recorded messages form one continuous conversation](after.jpg)

Earlier context includes stored root turns from the same session and trace owner that the caller can read. It excludes the selected run and later turns, and does not include a previous run's output if that run finished after the selected run started. It does not fetch Slack messages or reconstruct content omitted before ingestion
