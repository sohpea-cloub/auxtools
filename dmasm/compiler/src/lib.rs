pub mod compiler;
pub mod diagnostic;
pub mod syntax;
mod target;

#[cfg(test)]
use dmasm_bytecode::assembler;
use dmasm_bytecode::{
    operands, Instruction, Node, PickSwitchParams, SwitchParams, SwitchRangeParams, TypeFilter,
    Value,
};
pub use target::ByondVersion;

#[cfg(test)]
pub(crate) struct TestAssembleEnv;

#[cfg(test)]
impl assembler::AssembleEnv for TestAssembleEnv {
    fn get_string_index(&mut self, _data: &[u8]) -> Option<u32> {
        Some(1337)
    }

    fn get_variable_name_index(&mut self, _name: &[u8]) -> Option<u32> {
        Some(1338)
    }

    fn get_proc_index(&mut self, _path: &str) -> Option<u32> {
        Some(1339)
    }

    fn get_type(&mut self, _path: &str) -> Option<(u8, u32)> {
        Some((0x09, 0x01))
    }
}
