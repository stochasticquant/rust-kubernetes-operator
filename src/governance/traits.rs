use super::policies::Policy;

pub trait PolicyEvaluator {
    fn evaluate(&self) -> bool;
}

impl PolicyEvaluator for Policy {
    fn evaluate(&self) -> bool {
        self.enabled
    }
}