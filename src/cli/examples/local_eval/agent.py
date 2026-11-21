from collections.abc import Mapping
from types import MappingProxyType
from typing import Final

import litellm

litellm.suppress_debug_info = True

INSTRUCTIONS: Final = (
    "You triage customer support tickets. Classify each ticket as exactly one of: billing, technical, sales. "
    "When a ticket is referenced only by id, call lookup_ticket to read it first. "
    "Reply with only the category word."
)

TICKETS: Final[Mapping[str, str]] = MappingProxyType(
    {
        "T-1001": "I was charged twice for my March invoice and need a refund.",
        "T-1002": "The desktop app crashes every time I open the settings page.",
        "T-1003": "We are a team of 40 and want a quote for the enterprise plan.",
        "T-1004": "My card was declined but the money still left my account.",
        "T-1005": "SSO login fails with error 502 since this morning.",
        "T-1006": "Can we get volume pricing if we add another 25 seats?",
        "T-1007": "The export to CSV button does nothing in Firefox.",
        "T-1008": "Please update the VAT number printed on our invoices.",
    }
)


def lookup_ticket(ticket_id: str) -> str:
    """Read the message for a support ticket id."""
    return TICKETS.get(ticket_id.strip().upper(), f"No ticket named {ticket_id}")
