# Gear bridges

implementation of zk-based bridge to ethereum for gear-based blockchains

## Circuits

#### block finality
the basic building block for two types of proofs used in protocol

![block finality circuit](https://github.com/mertwole/gear-bridges/blob/main/images/block_finality_circuit.png)

#### validator set change
used to keep actual validator set on ethereum

![validator set change circuit](https://github.com/mertwole/gear-bridges/blob/main/images/next_validator_set_circuit.png)

#### recent validator set
used to prove chain of validator set changes to prove transition from genesis to the recent validator set in one proof

![recent validator set circuit](https://github.com/mertwole/gear-bridges/blob/main/images/recent_validator_set_circuit.png)

#### message sent
used to prove that specific message was submitted on gear chain for bridging

![message sent circuit](https://github.com/mertwole/gear-bridges/blob/main/images/message_sent_circuit.png)

#### final proof
proof that's submitted to ethereum

![final proof circuit](https://github.com/mertwole/gear-bridges/blob/main/images/final_proof_circuit.png)
