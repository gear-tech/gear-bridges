// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";

/**
 * @dev Mock NewImplementation smart contract is responsible for
 *      being implementation of UUPSUpgradeable contract.
 *      It is used for testing purposes.
 */
contract NewImplementationMock is UUPSUpgradeable {
    /**
     * @dev Function that should revert when `msg.sender` is not authorized to upgrade the contract.
     *      Called by {upgradeToAndCall}.
     */
    function _authorizeUpgrade(address newImplementation) internal override {}
}
