#![no_main]
#![no_std]

use vexide::prelude::*;

#[allow(
    non_snake_case,
    non_camel_case_types,
    non_upper_case_globals,
    improper_ctypes,
    unsafe_op_in_unsafe_fn,
    clippy::all
)]
pub mod ffmpeg {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

struct Robot {}

impl Compete for Robot {
    async fn autonomous(&mut self) {
        println!("Autonomous!");
    }

    async fn driver(&mut self) {
        println!("Driver!");
    }
}

#[vexide::main]
async fn main(peripherals: Peripherals) {
    let robot = Robot {};

    robot.compete().await;
}
