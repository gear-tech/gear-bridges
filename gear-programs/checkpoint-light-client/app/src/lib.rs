#![no_std]

use sails_rs::prelude::*;

struct CheckpointLightClientService(());

#[sails_rs::service]
impl CheckpointLightClientService {
    pub fn new() -> Self {
        Self(())
    }

    // Service's method (command)
    pub fn do_something(&mut self) -> String {
        "Hello from CheckpointLightClient!".to_string()
    }

    // Service's query
    pub fn get_something(&self) -> String {
        "Hello from CheckpointLightClient!".to_string()
    }    
}

pub struct CheckpointLightClientProgram(());

#[sails_rs::program]
impl CheckpointLightClientProgram {
    // Program's constructor
    pub fn new() -> Self {
        Self(())
    }

    // Exposed service
    pub fn checkpoint_light_client(&self) -> CheckpointLightClientService {
        CheckpointLightClientService::new()
    }
}
