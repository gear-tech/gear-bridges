# Gear bridges

implementation of zk-based bridge to ethereum for gear-based blockchains

## Circuits

#### block finality
the basic building block for two types of proofs used in protocol

![block finality circuit](https://github.com/mertwole/gear-bridges/blob/main/images/block_finality_circuit.png)

#### validator set change
used to keep actual validator set on ethereum

![validator set change circuit](https://github.com/mertwole/gear-bridges/blob/main/images/next_validator_set_circuit.png)

#### message sent
used to prove that specific message was submitted on gear chain for bridging

![message sent circuit](https://github.com/mertwole/gear-bridges/blob/main/images/message_sent_circuit.png)
