use std::{env, path::PathBuf};

use schema_rust_next::{
    MetaListenerTier, NexusDaemonShape, SocketModeBits, WorkingListenerTier,
    build::{GenerationDriver, GenerationPlan, ModuleEmission},
};

const SUPERVISION_SOCKET_MODE: u32 = 0o600;

fn main() {
    SchemaBuild::from_environment().run();
}

struct SchemaBuild {
    crate_root: PathBuf,
}

impl SchemaBuild {
    fn from_environment() -> Self {
        Self {
            crate_root: PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir set")),
        }
    }

    fn run(&self) {
        println!("cargo:rerun-if-changed=schema/nexus.schema");
        println!("cargo:rerun-if-changed=src/schema/daemon.rs");

        let plan = GenerationPlan::new(&self.crate_root, "system", "0.1.0")
            .with_module(ModuleEmission::daemon_module("nexus", Self::daemon_shape()));
        GenerationDriver::new(plan)
            .generate()
            .expect("generate system schema artifacts")
            .write_or_check("SYSTEM_UPDATE_SCHEMA_ARTIFACTS")
            .expect("checked-in system schema artifacts are fresh");
    }

    fn daemon_shape() -> NexusDaemonShape {
        NexusDaemonShape::new("system-daemon", WorkingListenerTier::component_decoded())
            .with_meta_tier(MetaListenerTier::new(SocketModeBits::new(
                SUPERVISION_SOCKET_MODE,
            )))
    }
}
