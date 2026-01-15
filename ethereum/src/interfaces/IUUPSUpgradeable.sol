// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.33;

/**
 * @dev Interface for the UUPSUpgradeable contract.
 */
interface IUUPSUpgradeable {
    /**
     * @dev Upgrades the implementation of the contract.
     */
    function upgradeToAndCall(address newImplementation, bytes calldata data) external payable;
}
