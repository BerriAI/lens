use crate::datasets::validation::{self, Field, Kind};
use crate::investigations::validation::{Kind as InvestigationKind, Model as InvestigationModel};

#[derive(Clone, Copy)]
pub(crate) enum Model {
    Preview,
    Selection,
}

const STRING: Kind = Kind::String(0, None);

impl Model {
    pub(crate) fn fields(self) -> &'static [Field] {
        match self {
            Self::Preview => &[
                ("as_of", false, Kind::AwareDatetime),
                ("offset", false, Kind::Unsigned),
                (
                    "selection",
                    true,
                    Kind::Record(validation::Model::Activity(Self::Selection)),
                ),
                (
                    "lookback_hours",
                    false,
                    Kind::Investigation(InvestigationKind::Lookback),
                ),
            ],
            Self::Selection => &[
                (
                    "source",
                    false,
                    Kind::Investigation(InvestigationKind::Literal(&[
                        "traces", "requests", "both",
                    ])),
                ),
                ("service", false, STRING),
                ("agent_name", false, STRING),
                (
                    "filters",
                    false,
                    Kind::Tuple(
                        &Kind::Record(validation::Model::Investigation(InvestigationModel::Filter)),
                        0,
                    ),
                ),
                (
                    "sample_size",
                    false,
                    Kind::Optional(&Kind::Investigation(InvestigationKind::PositiveInteger)),
                ),
                (
                    "sample_percent",
                    false,
                    Kind::Investigation(InvestigationKind::Number(Some(100.0))),
                ),
                ("team_id", false, STRING),
                ("execution_ids", false, Kind::Tuple(&STRING, 0)),
            ],
        }
    }
}
