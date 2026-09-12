// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

/**
 * @dev Governance constants.
 */
library GovernanceConstants {
    /**
     * @dev Change governance message discriminant.
     */
    uint256 internal constant CHANGE_GOVERNANCE = 0x00;
    /**
     * @dev Pause proxy message discriminant.
     */
    uint256 internal constant PAUSE_PROXY = 0x01;
    /**
     * @dev Unpause proxy message discriminant.
     */
    uint256 internal constant UNPAUSE_PROXY = 0x02;
    /**
     * @dev Upgrade proxy message discriminant.
     */
    uint256 internal constant UPGRADE_PROXY = 0x03;

    /**
     * @dev `uint8 discriminant` size.
     */
    uint256 internal constant DISCRIMINANT_SIZE = 1;
    /**
     * @dev `bytes32 newGovernance` size.
     */
    uint256 internal constant NEW_GOVERNANCE_SIZE = 32;
    /**
     * @dev `address proxy` size.
     */
    uint256 internal constant PROXY_ADDRESS_SIZE = 20;
    /**
     * @dev `address newImplementation` size.
     */
    uint256 internal constant NEW_IMPLEMENTATION_SIZE = 20;

    /**
     * @dev `DISCRIMINANT_SIZE` offset.
     */
    uint256 internal constant OFFSET1 = 1;
    /**
     * @dev `DISCRIMINANT_SIZE + PROXY_ADDRESS_SIZE` offset.
     */
    uint256 internal constant OFFSET2 = 21;
    /**
     * @dev `DISCRIMINANT_SIZE + PROXY_ADDRESS_SIZE + NEW_IMPLEMENTATION_SIZE` offset.
     */
    uint256 internal constant OFFSET3 = 41;

    /**
     * @dev `DISCRIMINANT_SIZE + NEW_GOVERNANCE_SIZE` size.
     */
    uint256 internal constant CHANGE_GOVERNANCE_SIZE = 33;
    /**
     * @dev `DISCRIMINANT_SIZE + PROXY_ADDRESS_SIZE` size.
     */
    uint256 internal constant PAUSE_UNPAUSE_PROXY_SIZE = 21;
    /**
     * @dev `DISCRIMINANT_SIZE + PROXY_ADDRESS_SIZE + NEW_IMPLEMENTATION_SIZE` size.
     */
    uint256 internal constant UPGRADE_PROXY_SIZE = 41;
}
