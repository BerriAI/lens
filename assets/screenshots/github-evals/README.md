# GitHub setup browser evidence

Captured from the Lens UI on October 9, 2026 at a 1440 × 1000 desktop viewport and a 390 × 844 mobile viewport

`before-agents.png` shows the empty authenticated directory before the change. `before-agents-demo.png` shows the existing demo directory. `before-evals.png` shows the existing empty eval view

The after screenshots use a real local Lens server and an isolated ClickHouse database. The agent, trace, dataset and eval named `github-flow-qa` are synthetic QA fixtures created through the public APIs. The Lens address entered in the setup form is an example, not a verified deployment. No model provider or external GitHub workflow was called, and no PR comment was posted

Open Agents, choose Connect GitHub for `github-flow-qa`, enter `BerriAI/lens` and a reachable Lens URL, then choose Continue. The workflow and credential screenshots show the two portions of the setup step. The verification screenshots show the honest waiting state before a PR eval arrives

The browser exercise covered the agent entry, eval selection, generated files, credential links, verification refresh and mobile layout. Integration tests cover matching GitHub run identity, running and failed evals, completed gate results, recovery from request failure, dataset prerequisites and navigation to the selected eval run
