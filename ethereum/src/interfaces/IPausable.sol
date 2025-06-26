// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

/**
 * @dev Interface for the pausable contracts.
 */
interface IPausable {
    /**
     * @dev Pauses the contract.
     */
    function pause() external;

    /**
     * @dev Unpauses the contract.
     */
    function unpause() external;
}
