// Copyright (C) Gear Technologies Inc.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {GovernanceControl} from "src/interfaces/IMessageQueue.sol";

struct ChangeGovernanceMessage {
    bytes32 newGovernance;
}

struct PauseProxyMessage {
    address proxy;
}

struct UnpauseProxyMessage {
    address proxy;
}

struct UpgradeProxyMessage {
    address proxy;
    address newImplementation;
    bytes data;
}

/// @dev Always packs the canonical versioned capability; retained handlers receive only its validated inner body.
library GovernancePacker {
    function envelope(GovernanceControl memory control, uint8 action, address target, bytes memory innerPayload)
        internal
        pure
        returns (bytes memory)
    {
        require(control.sequence != 0 && control.validUntil != 0, "control sequence/expiry required");
        require(action <= 4 && target != address(0), "invalid control action/target");
        require(
            action == 2
                || (control.incidentEpoch == 0
                    && control.requiredAdminAction == bytes32(0)
                    && control.requiredBarrier == bytes32(0)),
            "irrelevant control dependency"
        );
        return bytes.concat(
            hex"ff",
            abi.encode(
                uint8(1),
                control.sequence,
                control.validUntil,
                action,
                target,
                control.incidentEpoch,
                control.requiredAdminAction,
                control.requiredBarrier,
                innerPayload
            )
        );
    }

    function pack(ChangeGovernanceMessage memory message, GovernanceControl memory control, address handler)
        internal
        pure
        returns (bytes memory)
    {
        return envelope(control, 0, handler, abi.encodePacked(uint8(0), message.newGovernance));
    }

    function pack(PauseProxyMessage memory message, GovernanceControl memory control)
        internal
        pure
        returns (bytes memory)
    {
        return envelope(control, 1, message.proxy, abi.encodePacked(uint8(1), message.proxy));
    }

    function pack(UnpauseProxyMessage memory message, GovernanceControl memory control)
        internal
        pure
        returns (bytes memory)
    {
        require(control.requiredAdminAction != bytes32(0), "completed admin action required");
        return envelope(control, 2, message.proxy, abi.encodePacked(uint8(2), message.proxy));
    }

    function pack(UpgradeProxyMessage memory message, GovernanceControl memory control)
        internal
        pure
        returns (bytes memory)
    {
        return envelope(
            control,
            3,
            message.proxy,
            abi.encodePacked(uint8(3), message.proxy, message.newImplementation, message.data)
        );
    }

    function cancel(
        GovernanceControl memory control,
        address queue,
        bytes32 source,
        address destination,
        uint64 through
    ) internal pure returns (bytes memory) {
        return envelope(control, 4, queue, abi.encode(source, destination, through));
    }
}
