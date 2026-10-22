// Copyright (C) Gear Technologies Inc.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.38;

import {BeefyClient} from "src/beefy/BeefyClient.sol";

/// @dev Unit-test checkpoint admission only; inherited MMR verification remains real.
contract BeefyClientMock is BeefyClient {
    constructor(
        bytes32 sourceDomain_,
        uint256 destinationChainId_,
        address destinationQueue_,
        uint64 mmrStartBlock_,
        uint64 initialBeefyBlock_,
        uint64 initialSourceTimestampMs_,
        ValidatorSet memory initialValidatorSet_,
        ValidatorSet memory nextValidatorSet_
    )
        BeefyClient(
            sourceDomain_,
            destinationChainId_,
            destinationQueue_,
            mmrStartBlock_,
            initialBeefyBlock_,
            initialSourceTimestampMs_,
            initialValidatorSet_,
            nextValidatorSet_
        )
    {}

    function setCheckpoint(uint64 blockNumber, bytes32 mmrRoot, uint64 sourceTimestampMs) external {
        latestBeefyBlock = blockNumber;
        latestMMRRoot = mmrRoot;
        lastAuthenticatedSourceTimestampMs = sourceTimestampMs;
    }
}
