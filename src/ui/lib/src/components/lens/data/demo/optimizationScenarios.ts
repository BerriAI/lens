import type { Scenario } from "./scenarios";

export const optimizationScenarios: Scenario[] = Array.from({ length: 48 }, (_, index) => {
  const order = 2000 + index;
  const product = ["headphones", "keyboard", "monitor", "camera", "speakers", "tablet"][index % 6];
  const days = 3 + (index % 25);
  const examples = [
    {
      question: `Can I return the ${product} from order #${order}?`,
      tool: "lookup_return_policy",
      result: `Order #${order} was delivered ${days} days ago. Returns are free within 30 days. Original packaging is required.`,
      answer: `Yes, your ${product} is within the 30-day return window. Keep the original packaging; I can help create a free return label.`,
    },
    {
      question: `Where is order #${order} for my ${product}?`,
      tool: "lookup_order",
      result: `Order #${order} is in transit and due Friday. Tracking has not reported a delay.`,
      answer: `Your ${product} is in transit and expected Friday. The carrier has not reported any delays.`,
    },
    {
      question: `Has the refund for order #${order} arrived?`,
      tool: "lookup_refund",
      result: `Refund for order #${order} was issued yesterday. Bank processing takes 3–5 business days.`,
      answer: `Your refund was issued yesterday. It should appear on your original payment method within 3–5 business days.`,
    },
    {
      question: `Change the delivery address for order #${order}`,
      tool: "lookup_order",
      result: `Order #${order} is awaiting dispatch. Address changes require customer confirmation.`,
      answer: `Your order has not shipped yet. Please confirm the new delivery address so I can update it.`,
    },
    {
      question: `My ${product} in order #${order} arrived damaged`,
      tool: "lookup_replacement_policy",
      result: `Order #${order} qualifies for free replacement. Request a photo of the damaged item before creating a replacement.`,
      answer: `I’m sorry your ${product} arrived damaged. You qualify for a free replacement. Please share a photo of the damage so I can arrange it.`,
    },
    {
      question: `Cancel order #${order} for the ${product}`,
      tool: "lookup_order",
      result: `Order #${order} has shipped. Cancellation is unavailable; a free return can be started after delivery.`,
      answer: `Your order has already shipped, so I can’t cancel it. Once it arrives, I can help you start a free return.`,
    },
  ];
  return {
    agent: "support_agent",
    taskCategory: "Workflows",
    ...examples[index % examples.length],
  };
});

export const agentOptimizationScenarios: Scenario[] = [
  ...Array.from({ length: 32 }, (_, index): Scenario => {
    const area = ["billing", "search", "checkout", "authentication", "notifications", "exports", "settings", "reports"][
      index % 8
    ];
    const total = 120 + index * 3;
    const failed = index % 8 === 6 ? 2 : 0;
    return {
      agent: "release_agent",
      taskCategory: "Coding",
      question: `Review ${area} release v${2 + Math.floor(index / 8)}.${index % 8}`,
      tool: "read_release_checks",
      result: `${area}: ${total - failed} passed, ${failed} failed. Security review approved. Rollback plan verified.${failed ? " Failures affect empty-result handling. Hold until repaired." : " No blocking changes."}`,
      answer: failed
        ? `Hold this release. Two checks fail on empty-result handling. Repair those cases and rerun the checks before shipping.`
        : `The ${area} release is ready. All ${total} checks pass, the security review is approved, and the rollback plan is verified.`,
    };
  }),
  ...Array.from({ length: 32 }, (_, index): Scenario => {
    const topic = ["API retries", "audit logs", "data residency", "rate limits"][index % 4];
    const policy = [
      "Use exponential backoff after 429 and 503 responses. Do not retry non-idempotent operations without an idempotency key.",
      "Audit logs are retained for 90 days and can be exported before expiry.",
      "European storage is available on the business plan, subject to region selection before provisioning.",
      "Each team has a shared request budget. A 429 response includes a Retry-After header.",
    ][index % 4];
    return {
      agent: "research_agent",
      taskCategory: "Research",
      question: `Summarize ${topic} for integration ${index + 1}`,
      tool: "search_docs",
      result: policy,
      answer: `${policy} This is the documented behavior for this integration.`,
    };
  }),
  ...Array.from({ length: 36 }, (_, index): Scenario => {
    const seats = 10 + index * 5;
    const trial = index % 4 === 3;
    return {
      agent: "sales_agent",
      taskCategory: "Workflows",
      question: trial
        ? `Can our ${seats}-person team trial the business plan?`
        : `Which plan fits a team of ${seats} with SSO?`,
      tool: "lookup_plans",
      result: `Business supports SSO, audit logs, and ${seats} seats. Pricing is $20 per seat per month. Trials last 14 days and require no payment information. Annual discounts require sales approval.`,
      answer: trial
        ? `Your ${seats}-person team can start a 14-day business trial without entering payment information. It includes SSO and audit logs.`
        : `The business plan includes SSO and audit logs for your ${seats}-person team. At $20 per seat per month, the total is $${seats * 20} per month. Annual discounts require sales approval.`,
    };
  }),
];
