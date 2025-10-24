// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

interface ITetherToken {
    function issue(uint256 amount) external;

    function approve(address spender, uint256 value) external;
}
