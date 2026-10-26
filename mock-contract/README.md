# mock-contract

Simple contract which you can modify to include mock data for your tests. 

Used only by integration tests to supply the Holesky network and fixed recorded checkpoint pairs, including the historical-proxy receipt at slot 2,498,456 anchored at 2,498,464. These canned replies are not independent consensus verification.

Empty initialization retains those historical pairs. Owned native cutover tests may instead initialize with an exact SCALE `Vec<(u64, (u64, H256))>` of requested slots and checkpoint slot/root pairs; malformed/trailing bytes, empty lists, zero roots and duplicate requested slots are rejected. These explicit pairs take precedence over historical fixtures through the normal `CheckpointFor::Get` ABI. They authenticate the fixture's selected full-block bytes only, not Beacon consensus or mainnet finality.