// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

interface ICircleToken {
    function configureMinter(address minter, uint256 minterAllowedAmount) external;
}
