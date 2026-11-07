use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvePhase {
    CheckingLink,
    CheckingCache,
    StartingEngine,
    LoadingConfiguration,
    ResolvingLink,
    ConnectingAccount,
    FetchingMetadata,
    SelectingFormats,
    LoadingPreview,
    LoadingAuthor,
    Finalizing,
    CacheHit,
    Completed,
}

impl ResolvePhase {
    pub fn engine_order(self) -> Option<u8> {
        Some(match self {
            Self::LoadingConfiguration => 0,
            Self::ResolvingLink => 1,
            Self::FetchingMetadata => 2,
            Self::SelectingFormats => 3,
            Self::LoadingPreview => 4,
            Self::LoadingAuthor => 5,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolveProgress {
    pub phase: ResolvePhase,
    pub steps: Vec<ResolvePhase>,
}

impl ResolveProgress {
    pub fn new(phase: ResolvePhase) -> Self {
        Self { phase, steps: vec![phase] }
    }

    pub fn advance(&mut self, phase: ResolvePhase) {
        self.phase = phase;
        if !self.steps.contains(&phase) { self.steps.push(phase); }
    }
}
