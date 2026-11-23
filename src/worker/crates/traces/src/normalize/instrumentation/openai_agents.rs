use super::{ObservationType, RoleEvidence, SpanContext, SpanFacts};

pub(super) fn adjust(context: &SpanContext<'_>, facts: SpanFacts) -> SpanFacts {
    let workflow = context.parent_span_id.is_empty()
        && facts.role == Some(RoleEvidence::Declared(ObservationType::Agent))
        && facts.agent_name.is_none()
        && !context.attributes.contains_key("graph.node.id");
    if !workflow {
        return facts;
    }
    SpanFacts {
        role: Some(RoleEvidence::WrapperCandidate(ObservationType::Agent)),
        ..facts
    }
}
