// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

contract RecoverySafeTestWallet {
    uint256 private immutable _threshold;
    uint256 private immutable _ownerCount;

    constructor(uint256 threshold_, uint256 ownerCount_) {
        _threshold = threshold_;
        _ownerCount = ownerCount_;
    }

    function getThreshold() external view returns (uint256) {
        return _threshold;
    }

    function getOwners() external view returns (address[] memory owners) {
        owners = new address[](_ownerCount);
        for (uint256 i; i < _ownerCount; i++) {
            owners[i] = address(uint160(i + 1));
        }
    }
}
