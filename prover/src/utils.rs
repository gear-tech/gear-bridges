use plonky2::{
    field::extension::Extendable,
    hash::hash_types::RichField,
    iop::{
        generator::{GeneratedValues, SimpleGenerator},
        target::Target,
        witness::{PartitionWitness, Witness, WitnessWrite},
    },
    plonk::circuit_data::CommonCircuitData,
    util::serialization::{Buffer, IoResult, Read, Write},
};

#[derive(Debug, Default)]
pub struct CanonicalizeGenerator {
    pub(crate) target: Target,
    pub(crate) target_out: Target,
}

impl<F: RichField + Extendable<D>, const D: usize> SimpleGenerator<F, D> for CanonicalizeGenerator {
    fn id(&self) -> String {
        "CanonicalizeGenerator".to_string()
    }

    fn dependencies(&self) -> Vec<Target> {
        vec![self.target]
    }

    fn run_once(&self, witness: &PartitionWitness<F>, out_buffer: &mut GeneratedValues<F>) {
        let value = witness.get_target(self.target);

        out_buffer.set_target(self.target_out, value.to_canonical());
    }

    fn serialize(&self, dst: &mut Vec<u8>, _common_data: &CommonCircuitData<F, D>) -> IoResult<()> {
        dst.write_target(self.target)?;
        dst.write_target(self.target_out)
    }

    fn deserialize(source: &mut Buffer, _common_data: &CommonCircuitData<F, D>) -> IoResult<Self> {
        let target = source.read_target()?;
        let target_out = source.read_target()?;

        Ok(Self { target, target_out })
    }
}
