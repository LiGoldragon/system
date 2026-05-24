use signal_system::SystemTarget;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessTarget {
    name: String,
    target: SystemTarget,
}

impl HarnessTarget {
    pub fn new(name: impl Into<String>, target: SystemTarget) -> Self {
        Self {
            name: name.into(),
            target,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn owns_target(&self, target: SystemTarget) -> bool {
        self.target == target
    }
}
