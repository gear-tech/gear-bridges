// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

/// @dev Optional verifier capability for advancing a queue through an authenticated empty snapshot.
interface IQueueProgressVerifier {
    function safeVerifyEmptyQueueProgress(uint256 sourceBlock, bytes calldata proof) external view returns (bool);
}
