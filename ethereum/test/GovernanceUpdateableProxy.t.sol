pragma solidity ^0.8.13;

import {Test} from "forge-std/Test.sol";
import {ERC20Treasury} from "../src/ERC20Treasury.sol";
import {GovernanceUpdateableProxy} from "../src/GovernanceUpdateableProxy.sol";
import {VFT_GATEWAY_ADDRESS} from "../src/libraries/Environment.sol";
import {ERC20Mock} from "../src/mocks/ERC20Mock.sol";
import {IERC20Errors} from "@openzeppelin/contracts/interfaces/draft-IERC6093.sol";

contract GovernanceUpdateableProxyTest is Test {
    GovernanceUpdateableProxy public proxy;
    ERC20Mock public erc20;

    address constant MESSAGE_QUEUE = address(10_000);
    address constant TOKEN_RECEIVER = address(10_001);

    bytes32 constant GOVERNANCE = bytes32("governance_governance_governance");
    bytes32 constant NOT_GOVERNANCE =
        bytes32("not_governance_governance_govern");

    function setUp() public {
        erc20 = new ERC20Mock("");

        ERC20Treasury treasury = new ERC20Treasury(MESSAGE_QUEUE);
        proxy = new GovernanceUpdateableProxy(
            address(treasury),
            MESSAGE_QUEUE,
            GOVERNANCE
        );
    }

    function test_callIsDelegated() public {
        vm.startPrank(MESSAGE_QUEUE, MESSAGE_QUEUE);

        address receiver = TOKEN_RECEIVER;
        address token = address(erc20);
        uint256 amount = 0;

        proxy.processVaraMessage(
            VFT_GATEWAY_ADDRESS,
            abi.encodePacked(uint8(0x00), receiver, token, amount)
        );

        amount = 1;

        // Test that revert data is returned unmangled.
        vm.expectRevert(
            abi.encodeWithSelector(
                IERC20Errors.ERC20InsufficientBalance.selector,
                address(0xF62849F9A0B5Bf2913b396098F7c7019b51A820a),
                0,
                amount
            )
        );
        proxy.processVaraMessage(
            VFT_GATEWAY_ADDRESS,
            abi.encodePacked(uint8(0x00), receiver, token, amount)
        );
    }

    function test_implementationIsUpdated() public {
        vm.startPrank(MESSAGE_QUEUE, MESSAGE_QUEUE);

        address newImplementation = address(0x1234);

        proxy.processVaraMessage(
            GOVERNANCE,
            abi.encodePacked(uint8(0x01), newImplementation)
        );

        assertEq(proxy.getImplementation(), newImplementation);
    }

    function test_governanceIsUpdated() public {
        vm.startPrank(MESSAGE_QUEUE, MESSAGE_QUEUE);

        proxy.processVaraMessage(
            GOVERNANCE,
            abi.encodePacked(uint8(0x02), NOT_GOVERNANCE)
        );

        assertEq(proxy.getGovernance(), NOT_GOVERNANCE);
    }

    function test_fakeGovernanceIsRejected() public {
        vm.startPrank(MESSAGE_QUEUE, MESSAGE_QUEUE);

        vm.expectRevert(
            GovernanceUpdateableProxy.ProxyDeniedAdminAccess.selector
        );
        proxy.processVaraMessage(
            NOT_GOVERNANCE,
            abi.encodePacked(uint8(0x01), address(0))
        );

        vm.expectRevert(
            GovernanceUpdateableProxy.ProxyDeniedAdminAccess.selector
        );
        proxy.processVaraMessage(
            NOT_GOVERNANCE,
            abi.encodePacked(uint8(0x02), bytes32(0))
        );
    }
}
