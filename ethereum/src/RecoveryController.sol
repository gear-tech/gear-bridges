// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

interface IRecoveryThresholdWallet {
    function getThreshold() external view returns (uint256);
    function getOwners() external view returns (address[] memory);
}

interface IRecoveryQueue {
    function verifier() external view returns (address);
    function activateRecoveryVerifier(address expectedOldVerifier, address candidateVerifier) external;
}

interface IRecoveryRootVerifier {
    function beefyClient() external view returns (address);
    function messageQueue() external view returns (address);
    function destinationChainId() external view returns (uint256);
}

interface IRecoveryBeefyClient {
    function minNumRequiredSignatures() external view returns (uint256);
    function fiatShamirRequiredSignatures() external view returns (uint256);
    function MAX_VALIDATORS() external view returns (uint256);
    function randaoCommitDelay() external view returns (uint256);
    function randaoCommitExpiration() external view returns (uint256);
    function isLive() external view returns (bool);
    function latestMMRRoot() external view returns (bytes32);
    function latestBeefyBlock() external view returns (uint64);
    function sourceDomain() external view returns (bytes32);
    function bridgeDomain() external view returns (bytes32);
    function destinationChainId() external view returns (uint256);
    function destinationQueue() external view returns (address);
    function mmrStartBlock() external view returns (uint64);
}

/// @dev Verifier-only recovery pinned to one queue and an externally trusted 3-of-5 wallet address.
contract RecoveryController {
    uint256 public constant RECOVERY_DELAY = 24 hours;

    error InvalidRecoveryQueue();
    error InvalidRecoveryWallet();
    error NotRecoveryWallet();
    error RecoveryAlreadyPending();
    error InvalidProposal();
    error OldVerifierChanged();
    error InvalidRootVerifier();
    error InvalidBeefyClient();
    error RecoveryIdentityMismatch();
    error RecoveryCodeChanged();
    error CandidateHasNoMMR();
    error CandidateNotForward();
    error CandidateNotLive();
    error OldClientStillLive();
    error TimelockNotElapsed();

    struct PendingRecovery {
        uint256 proposalId;
        address expectedOldVerifier;
        address candidateVerifier;
        uint256 executeAfter;
        bytes32 expectedOldVerifierCodeHash;
        address expectedOldClient;
        bytes32 expectedOldClientCodeHash;
        bytes32 candidateVerifierCodeHash;
        address candidateClient;
        bytes32 candidateClientCodeHash;
        bool exists;
    }

    IRecoveryQueue public immutable messageQueue;
    address public immutable recoveryWallet;
    uint256 public proposalNonce;
    PendingRecovery public pendingRecovery;

    event RecoveryProposed(
        uint256 indexed proposalId,
        address indexed expectedOldVerifier,
        address indexed candidateVerifier,
        uint256 executeAfter,
        bytes32 expectedOldVerifierCodeHash,
        address expectedOldClient,
        bytes32 expectedOldClientCodeHash,
        bytes32 candidateVerifierCodeHash,
        address candidateClient,
        bytes32 candidateClientCodeHash
    );
    event RecoveryCancelled(uint256 indexed proposalId);
    event RecoveryExecuted(uint256 indexed proposalId, address indexed previousVerifier, address indexed newVerifier);

    constructor(address messageQueue_, address recoveryWallet_) {
        if (messageQueue_ == address(0) || msg.sender != messageQueue_) revert InvalidRecoveryQueue();
        if (recoveryWallet_.code.length == 0 || recoveryWallet_ == messageQueue_) revert InvalidRecoveryWallet();
        messageQueue = IRecoveryQueue(messageQueue_);
        recoveryWallet = recoveryWallet_;
        _requireThreeOfFiveWallet();
    }

    modifier onlyRecoveryWallet() {
        if (msg.sender != recoveryWallet) revert NotRecoveryWallet();
        _;
    }

    function proposeRecovery(address expectedOldVerifier, address candidateVerifier)
        external
        onlyRecoveryWallet
        returns (uint256 proposalId)
    {
        if (pendingRecovery.exists) revert RecoveryAlreadyPending();
        _requireThreeOfFiveWallet();
        if (expectedOldVerifier == address(0) || messageQueue.verifier() != expectedOldVerifier) {
            revert OldVerifierChanged();
        }
        if (candidateVerifier == address(0) || candidateVerifier == expectedOldVerifier) {
            revert InvalidRootVerifier();
        }

        (IRecoveryBeefyClient oldClient, IRecoveryBeefyClient candidate) =
            _validateCandidate(expectedOldVerifier, candidateVerifier, false);

        proposalId = ++proposalNonce;
        uint256 executeAfter = block.timestamp + RECOVERY_DELAY;
        bytes32 oldVerifierCodeHash = expectedOldVerifier.codehash;
        bytes32 oldClientCodeHash = address(oldClient).codehash;
        bytes32 candidateVerifierCodeHash = candidateVerifier.codehash;
        bytes32 candidateClientCodeHash = address(candidate).codehash;
        pendingRecovery = PendingRecovery({
            proposalId: proposalId,
            expectedOldVerifier: expectedOldVerifier,
            candidateVerifier: candidateVerifier,
            executeAfter: executeAfter,
            expectedOldVerifierCodeHash: oldVerifierCodeHash,
            expectedOldClient: address(oldClient),
            expectedOldClientCodeHash: oldClientCodeHash,
            candidateVerifierCodeHash: candidateVerifierCodeHash,
            candidateClient: address(candidate),
            candidateClientCodeHash: candidateClientCodeHash,
            exists: true
        });
        emit RecoveryProposed(
            proposalId,
            expectedOldVerifier,
            candidateVerifier,
            executeAfter,
            oldVerifierCodeHash,
            address(oldClient),
            oldClientCodeHash,
            candidateVerifierCodeHash,
            address(candidate),
            candidateClientCodeHash
        );
    }

    function cancelRecovery(uint256 proposalId) external onlyRecoveryWallet {
        if (!pendingRecovery.exists || pendingRecovery.proposalId != proposalId) revert InvalidProposal();
        delete pendingRecovery;
        emit RecoveryCancelled(proposalId);
    }

    /// @dev Any account may execute the exact approved binding after the fixed delay.
    function executeRecovery(uint256 proposalId) external {
        PendingRecovery memory proposal = pendingRecovery;
        if (!proposal.exists || proposal.proposalId != proposalId) revert InvalidProposal();
        if (block.timestamp < proposal.executeAfter) revert TimelockNotElapsed();

        _requireThreeOfFiveWallet();
        if (
            proposal.expectedOldVerifier.codehash != proposal.expectedOldVerifierCodeHash
                || proposal.candidateVerifier.codehash != proposal.candidateVerifierCodeHash
                || IRecoveryRootVerifier(proposal.expectedOldVerifier).beefyClient() != proposal.expectedOldClient
                || IRecoveryRootVerifier(proposal.candidateVerifier).beefyClient() != proposal.candidateClient
                || proposal.expectedOldClient.codehash != proposal.expectedOldClientCodeHash
                || proposal.candidateClient.codehash != proposal.candidateClientCodeHash
        ) revert RecoveryCodeChanged();

        (IRecoveryBeefyClient oldClient, IRecoveryBeefyClient candidate) =
            _validateCandidate(proposal.expectedOldVerifier, proposal.candidateVerifier, true);
        if (address(oldClient) != proposal.expectedOldClient || address(candidate) != proposal.candidateClient) {
            revert RecoveryCodeChanged();
        }

        delete pendingRecovery;
        messageQueue.activateRecoveryVerifier(proposal.expectedOldVerifier, proposal.candidateVerifier);
        emit RecoveryExecuted(proposalId, proposal.expectedOldVerifier, proposal.candidateVerifier);
    }

    function _validateCandidate(address oldVerifier, address candidateVerifier, bool requireOldExpired)
        private
        view
        returns (IRecoveryBeefyClient oldClient, IRecoveryBeefyClient candidate)
    {
        if (messageQueue.verifier() != oldVerifier) revert OldVerifierChanged();

        oldClient = _clientForVerifier(oldVerifier);
        candidate = _clientForVerifier(candidateVerifier);

        if (
            oldClient.sourceDomain() == bytes32(0) || candidate.sourceDomain() != oldClient.sourceDomain()
                || candidate.bridgeDomain() != oldClient.bridgeDomain()
                || candidate.destinationChainId() != oldClient.destinationChainId()
                || candidate.destinationQueue() != oldClient.destinationQueue()
                || candidate.mmrStartBlock() != oldClient.mmrStartBlock()
        ) {
            revert RecoveryIdentityMismatch();
        }
        if (address(candidate).codehash != address(oldClient).codehash) revert RecoveryCodeChanged();
        if (candidate.latestBeefyBlock() <= oldClient.latestBeefyBlock()) revert CandidateNotForward();
        if (candidate.latestMMRRoot() == bytes32(0)) revert CandidateHasNoMMR();
        if (!candidate.isLive()) revert CandidateNotLive();
        if (requireOldExpired && oldClient.isLive()) revert OldClientStillLive();
    }

    function _clientForVerifier(address verifier) private view returns (IRecoveryBeefyClient client) {
        if (verifier.code.length == 0) revert InvalidRootVerifier();
        IRecoveryRootVerifier rootVerifier = IRecoveryRootVerifier(verifier);
        if (rootVerifier.messageQueue() != address(messageQueue) || rootVerifier.destinationChainId() != block.chainid)
        {
            revert InvalidRootVerifier();
        }

        address clientAddress = rootVerifier.beefyClient();
        if (clientAddress.code.length == 0) revert InvalidBeefyClient();
        client = IRecoveryBeefyClient(clientAddress);
        if (
            client.minNumRequiredSignatures() != 86 || client.fiatShamirRequiredSignatures() != 86
                || client.MAX_VALIDATORS() != 256 || client.randaoCommitDelay() != 128
                || client.randaoCommitExpiration() != 24
                || client.sourceDomain() == bytes32(0) || client.destinationChainId() != block.chainid
                || client.destinationQueue() != address(messageQueue) || client.mmrStartBlock() == 0
                || client.bridgeDomain()
                    != keccak256(
                        abi.encodePacked(
                            "vara/gear-eth-bridge-domain/v2",
                            client.sourceDomain(),
                            bytes32(block.chainid),
                            address(messageQueue)
                        )
                    )
        ) {
            revert InvalidBeefyClient();
        }
    }

    function _requireThreeOfFiveWallet() private view {
        uint256 threshold;
        address[] memory owners;
        try IRecoveryThresholdWallet(recoveryWallet).getThreshold() returns (uint256 value) {
            threshold = value;
        } catch {
            revert InvalidRecoveryWallet();
        }
        try IRecoveryThresholdWallet(recoveryWallet).getOwners() returns (address[] memory values) {
            owners = values;
        } catch {
            revert InvalidRecoveryWallet();
        }
        if (threshold != 3 || owners.length != 5) revert InvalidRecoveryWallet();
        for (uint256 i; i < owners.length; i++) {
            if (owners[i] == address(0)) revert InvalidRecoveryWallet();
            for (uint256 j; j < i; j++) {
                if (owners[i] == owners[j]) revert InvalidRecoveryWallet();
            }
        }
    }
}
