// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {CommonBase} from "forge-std/Base.sol";
import {console} from "forge-std/console.sol";
import {StdAssertions} from "forge-std/StdAssertions.sol";
import {StdChains} from "forge-std/StdChains.sol";
import {StdCheats} from "forge-std/StdCheats.sol";
import {StdInvariant} from "forge-std/StdInvariant.sol";
import {StdUtils} from "forge-std/StdUtils.sol";
import {IERC20Metadata} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Metadata.sol";
import {Upgrades} from "openzeppelin-foundry-upgrades/Upgrades.sol";
import {ERC20GearSupply} from "src/erc20/managed/ERC20GearSupply.sol";
import {CircleToken} from "src/erc20/CircleToken.sol";
import {TetherToken} from "src/erc20/TetherToken.sol";
import {WrappedEther} from "src/erc20/WrappedEther.sol";
import {WrappedVara} from "src/erc20/WrappedVara.sol";
import {IERC20Manager} from "src/interfaces/IERC20Manager.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";
import {MessageHandlerMock} from "src/mocks/MessageHandlerMock.sol";
import {NewImplementationMock} from "src/mocks/NewImplementationMock.sol";
import {VerifierMock} from "src/mocks/VerifierMock.sol";
import {BridgingPayment} from "src/BridgingPayment.sol";
import {ERC20Manager} from "src/ERC20Manager.sol";
import {GovernanceAdmin} from "src/GovernanceAdmin.sol";
import {GovernancePauser} from "src/GovernancePauser.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {Verifier} from "src/Verifier.sol";

struct Overrides {
    address circleToken;
    address tetherToken;
    address wrappedEther;
}

struct DeploymentArguments {
    uint256 privateKey;
    address deployerAddress;
    Overrides overrides;
    bytes32 vftManager;
    bytes32 governanceAdmin;
    bytes32 governancePauser;
    address emergencyStopAdmin;
    uint256 bridgingPaymentFee;
}

library BaseConstants {
    address internal constant ZERO_ADDRESS = address(0);
    uint256 internal constant DEPLOYER_INITIAL_BALANCE = 100 ether;
    address internal constant DEPLOYER_ADDRESS = 0x1111111111111111111111111111111111111111;
    bytes32 internal constant VFT_MANAGER = 0x2222222222222222222222222222222222222222222222222222222222222222;
    bytes32 internal constant GOVERNANCE_ADMIN = 0x3333333333333333333333333333333333333333333333333333333333333333;
    bytes32 internal constant GOVERNANCE_PAUSER = 0x4444444444444444444444444444444444444444444444444444444444444444;
    address internal constant EMERGENCY_STOP_ADMIN = 0x5555555555555555555555555555555555555555;
    uint256 internal constant BRIDGING_PAYMENT_FEE = 1 wei;
}

abstract contract Base is CommonBase, StdAssertions, StdChains, StdCheats, StdInvariant, StdUtils {
    DeploymentArguments public deploymentArguments;

    IERC20Metadata public erc20GearSupply;

    IERC20Metadata public circleToken;
    IERC20Metadata public tetherToken;
    IERC20Metadata public wrappedEther;
    IERC20Metadata public wrappedVara;

    GovernanceAdmin public governanceAdmin;
    GovernancePauser public governancePauser;

    IVerifier public verifier;
    MessageQueue public messageQueue;

    ERC20Manager public erc20Manager;

    BridgingPayment public bridgingPayment;

    MessageHandlerMock public messageHandlerMock;
    NewImplementationMock public newImplementationMock;

    function deployBridgeFromConstants() public {
        deployBridge(
            DeploymentArguments({
                privateKey: 0,
                deployerAddress: BaseConstants.DEPLOYER_ADDRESS,
                overrides: Overrides({
                    circleToken: BaseConstants.ZERO_ADDRESS,
                    tetherToken: BaseConstants.ZERO_ADDRESS,
                    wrappedEther: BaseConstants.ZERO_ADDRESS
                }),
                vftManager: BaseConstants.VFT_MANAGER,
                governanceAdmin: BaseConstants.GOVERNANCE_ADMIN,
                governancePauser: BaseConstants.GOVERNANCE_PAUSER,
                emergencyStopAdmin: BaseConstants.EMERGENCY_STOP_ADMIN,
                bridgingPaymentFee: BaseConstants.BRIDGING_PAYMENT_FEE
            })
        );
    }

    function deployBridgeFromEnvironment() public {
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        address deployerAddress = vm.addr(privateKey);

        deployBridge(
            DeploymentArguments({
                privateKey: privateKey,
                deployerAddress: deployerAddress,
                overrides: Overrides({
                    circleToken: vm.envExists("CIRCLE_TOKEN") ? vm.envAddress("CIRCLE_TOKEN") : BaseConstants.ZERO_ADDRESS,
                    tetherToken: vm.envExists("TETHER_TOKEN") ? vm.envAddress("TETHER_TOKEN") : BaseConstants.ZERO_ADDRESS,
                    wrappedEther: vm.envExists("WRAPPED_ETHER") ? vm.envAddress("WRAPPED_ETHER") : BaseConstants.ZERO_ADDRESS
                }),
                vftManager: vm.envBytes32("VFT_MANAGER"),
                governanceAdmin: vm.envBytes32("GOVERNANCE_ADMIN"),
                governancePauser: vm.envBytes32("GOVERNANCE_PAUSER"),
                emergencyStopAdmin: vm.envAddress("EMERGENCY_STOP_ADMIN"),
                bridgingPaymentFee: vm.envUint("BRIDGING_PAYMENT_FEE")
            })
        );
    }

    function deployBridge(DeploymentArguments memory _deploymentArguments) public {
        console.log("Deployment arguments:");

        deploymentArguments = _deploymentArguments;

        bool isTest = deploymentArguments.privateKey == 0;
        bool isScript = !isTest;

        console.log("    deployerAddress:     ", deploymentArguments.deployerAddress);
        console.log("    vftManager:          ", vm.toString(deploymentArguments.vftManager));
        console.log("    governanceAdmin:     ", vm.toString(deploymentArguments.governanceAdmin));
        console.log("    governancePauser:    ", vm.toString(deploymentArguments.governancePauser));
        console.log("    bridgingPaymentFee:  ", deploymentArguments.bridgingPaymentFee, "wei");

        if (isTest) {
            vm.deal(deploymentArguments.deployerAddress, BaseConstants.DEPLOYER_INITIAL_BALANCE);
            vm.startPrank(deploymentArguments.deployerAddress, deploymentArguments.deployerAddress);
        } else if (isScript) {
            vm.startBroadcast(deploymentArguments.privateKey);
        }

        console.log();

        //////////////////////////////////////////////////////////////////////////////

        console.log("ERC20 tokens:");

        // for verification purposes on Etherscan
        erc20GearSupply = new ERC20GearSupply(deploymentArguments.deployerAddress, "MyToken", "MTK", 18);

        if (isTest) {
            deployTestTokens();
        } else if (isScript) {
            if (shouldUseOverrides()) {
                circleToken = IERC20Metadata(deploymentArguments.overrides.circleToken);
                tetherToken = IERC20Metadata(deploymentArguments.overrides.tetherToken);
                wrappedEther = IERC20Metadata(deploymentArguments.overrides.wrappedEther);
            } else {
                deployTestTokens();
            }
        }

        console.log("    USDC:                ", address(circleToken));
        console.log("    USDT:                ", address(tetherToken));
        console.log("    WETH:                ", address(wrappedEther));

        address erc20ManagerAddress = vm.computeCreateAddress(
            deploymentArguments.deployerAddress, vm.getNonce(deploymentArguments.deployerAddress) + 7
        );

        wrappedVara = new WrappedVara(erc20ManagerAddress);
        console.log("    WVARA:               ", address(wrappedVara));

        console.log();

        //////////////////////////////////////////////////////////////////////////////

        console.log("Bridge governance:");

        address messageQueueAddress = vm.computeCreateAddress(
            deploymentArguments.deployerAddress, vm.getNonce(deploymentArguments.deployerAddress) + 4
        );

        governanceAdmin =
            new GovernanceAdmin(deploymentArguments.governanceAdmin, messageQueueAddress, erc20ManagerAddress);
        console.log("    GovernanceAdmin:     ", address(governanceAdmin));

        governancePauser =
            new GovernancePauser(deploymentArguments.governancePauser, messageQueueAddress, erc20ManagerAddress);
        console.log("    GovernancePauser:    ", address(governancePauser));

        console.log();

        //////////////////////////////////////////////////////////////////////////////

        console.log("Bridge core:");

        if (isTest) {
            verifier = new VerifierMock(true);
        } else if (isScript) {
            verifier = new Verifier();
        }

        console.log("    Verifier:            ", address(verifier));

        // TODO: `npm warn exec The following package was not found and will be installed: @openzeppelin/upgrades-core@x.y.z`
        messageQueue = MessageQueue(
            Upgrades.deployUUPSProxy(
                "MessageQueue.sol",
                abi.encodeCall(
                    MessageQueue.initialize,
                    (governanceAdmin, governancePauser, deploymentArguments.emergencyStopAdmin, verifier)
                )
            )
        );
        console.log("    MessageQueue:        ", address(messageQueue));

        assertEq(messageQueueAddress, address(messageQueue));
        assertEq(messageQueue.governanceAdmin(), address(governanceAdmin));
        assertEq(messageQueue.governancePauser(), address(governancePauser));
        assertEq(messageQueue.emergencyStopAdmin(), deploymentArguments.emergencyStopAdmin);
        assertEq(messageQueue.verifier(), address(verifier));
        assertEq(messageQueue.isEmergencyStopped(), false);

        console.log();

        //////////////////////////////////////////////////////////////////////////////

        console.log("Bridge:");

        IERC20Manager.TokenInfo[] memory tokens = new IERC20Manager.TokenInfo[](4);

        tokens[0] = IERC20Manager.TokenInfo(address(circleToken), IERC20Manager.TokenType.Ethereum);
        tokens[1] = IERC20Manager.TokenInfo(address(tetherToken), IERC20Manager.TokenType.Ethereum);
        tokens[2] = IERC20Manager.TokenInfo(address(wrappedEther), IERC20Manager.TokenType.Ethereum);
        tokens[3] = IERC20Manager.TokenInfo(address(wrappedVara), IERC20Manager.TokenType.Gear);

        erc20Manager = ERC20Manager(
            Upgrades.deployUUPSProxy(
                "ERC20Manager.sol",
                abi.encodeCall(
                    ERC20Manager.initialize,
                    (governanceAdmin, governancePauser, address(messageQueue), deploymentArguments.vftManager, tokens)
                )
            )
        );
        console.log("    ERC20Manager:        ", address(erc20Manager));

        erc20ManagerAssertions(address(erc20Manager));

        //////////////////////////////////////////////////////////////////////////////

        console.log("Bridging payment:");

        bridgingPayment = BridgingPayment(erc20Manager.createBridgingPayment(deploymentArguments.bridgingPaymentFee));
        console.log("    BridgingPayment:     ", address(bridgingPayment));

        bridgingPaymentAssertions();

        //////////////////////////////////////////////////////////////////////////////

        if (isTest) {
            console.log();

            console.log("Test specific:");

            messageHandlerMock = new MessageHandlerMock();
            console.log("    MessageHandlerMock:  ", address(messageHandlerMock));

            newImplementationMock = new NewImplementationMock();
            console.log("    NewImplementationMock:", address(newImplementationMock));
        } else if (isScript) {
            console.log();

            console.log("Script specific:");

            printContractInfo(
                "MessageQueue", address(messageQueue), Upgrades.getImplementationAddress(address(messageQueue))
            );
            printContractInfo(
                "ERC20Manager", address(erc20Manager), Upgrades.getImplementationAddress(address(erc20Manager))
            );
        }

        //////////////////////////////////////////////////////////////////////////////

        if (isTest) {
            vm.stopPrank();
        } else if (isScript) {
            vm.stopBroadcast();
        }
    }

    function deployTestTokens() public {
        circleToken = new CircleToken(deploymentArguments.deployerAddress);
        tetherToken = new TetherToken(deploymentArguments.deployerAddress);
        wrappedEther = new WrappedEther();
    }

    function shouldUseOverrides() public view returns (bool) {
        return deploymentArguments.overrides.circleToken != BaseConstants.ZERO_ADDRESS
            && deploymentArguments.overrides.tetherToken != BaseConstants.ZERO_ADDRESS
            && deploymentArguments.overrides.wrappedEther != BaseConstants.ZERO_ADDRESS;
    }

    function erc20ManagerAssertions(address erc20ManagerAddress) public view {
        assertEq(erc20ManagerAddress, address(erc20Manager));
        assertEq(erc20Manager.governanceAdmin(), address(governanceAdmin));
        assertEq(erc20Manager.governancePauser(), address(governancePauser));
        assertEq(erc20Manager.messageQueue(), address(messageQueue));
        assertEq(erc20Manager.totalVftManagers(), 1);
        bytes32[] memory vftManagers1 = erc20Manager.vftManagers();
        assertEq(vftManagers1.length, 1);
        assertEq(vftManagers1[0], deploymentArguments.vftManager);
        bytes32[] memory vftManagers2 = erc20Manager.vftManagers(1, 1);
        assertEq(vftManagers2.length, 0);
        bytes32[] memory vftManagers3 = erc20Manager.vftManagers(0, 1);
        assertEq(vftManagers3.length, 1);
        assertEq(vftManagers3[0], deploymentArguments.vftManager);
        bytes32[] memory vftManagers4 = erc20Manager.vftManagers(0, 5);
        assertEq(vftManagers4.length, 1);
        assertEq(vftManagers4[0], deploymentArguments.vftManager);
        assertTrue(erc20Manager.isVftManager(deploymentArguments.vftManager));
        assertEq(erc20Manager.totalTokens(), 4);
        address[] memory tokens1 = erc20Manager.tokens();
        assertEq(tokens1.length, 4);
        assertEq(tokens1[0], address(circleToken));
        assertEq(tokens1[1], address(tetherToken));
        assertEq(tokens1[2], address(wrappedEther));
        assertEq(tokens1[3], address(wrappedVara));
        address[] memory tokens2 = erc20Manager.tokens(4, 4);
        assertEq(tokens2.length, 0);
        address[] memory tokens3 = erc20Manager.tokens(0, 2);
        assertEq(tokens3.length, 2);
        assertEq(tokens3[0], address(circleToken));
        assertEq(tokens3[1], address(tetherToken));
        address[] memory tokens4 = erc20Manager.tokens(2, 2);
        assertEq(tokens4.length, 2);
        assertEq(tokens4[0], address(wrappedEther));
        assertEq(tokens4[1], address(wrappedVara));
        address[] memory tokens5 = erc20Manager.tokens(0, 5);
        assertEq(tokens5.length, 4);
        assertEq(tokens5[0], address(circleToken));
        assertEq(tokens5[1], address(tetherToken));
        assertEq(tokens5[2], address(wrappedEther));
        assertEq(tokens5[3], address(wrappedVara));
        assertTrue(erc20Manager.getTokenType(address(circleToken)) == IERC20Manager.TokenType.Ethereum);
        assertTrue(erc20Manager.getTokenType(address(tetherToken)) == IERC20Manager.TokenType.Ethereum);
        assertTrue(erc20Manager.getTokenType(address(wrappedEther)) == IERC20Manager.TokenType.Ethereum);
        assertTrue(erc20Manager.getTokenType(address(wrappedVara)) == IERC20Manager.TokenType.Gear);
        assertTrue(erc20Manager.getTokenType(address(0)) == IERC20Manager.TokenType.Unknown);
    }

    function bridgingPaymentAssertions() public view {
        assertEq(bridgingPayment.erc20Manager(), address(erc20Manager));
        assertEq(erc20Manager.totalBridgingPayments(), 1);
        address[] memory bridgingPayments1 = erc20Manager.bridgingPayments();
        assertEq(bridgingPayments1.length, 1);
        assertEq(bridgingPayments1[0], address(bridgingPayment));
        address[] memory bridgingPayments2 = erc20Manager.bridgingPayments(1, 1);
        assertEq(bridgingPayments2.length, 0);
        address[] memory bridgingPayments3 = erc20Manager.bridgingPayments(0, 1);
        assertEq(bridgingPayments3.length, 1);
        assertEq(bridgingPayments3[0], address(bridgingPayment));
        address[] memory bridgingPayments4 = erc20Manager.bridgingPayments(0, 5);
        assertEq(bridgingPayments4.length, 1);
        assertEq(bridgingPayments4[0], address(bridgingPayment));
        assertFalse(erc20Manager.isBridgingPayment(address(0)));
        assertTrue(erc20Manager.isBridgingPayment(address(bridgingPayment)));
    }

    function printContractInfo(string memory contractName, address contractAddress, address expectedImplementation)
        public
        view
    {
        console.log("================================================================================================");
        console.log("[ CONTRACT  ]", contractName);
        console.log("[ ADDRESS   ]", contractAddress);
        if (expectedImplementation != address(0)) {
            console.log("[ IMPL ADDR ]", expectedImplementation);
            console.log(
                "[ PROXY VERIFICATION ] Click \"Is this a proxy?\" on Etherscan to be able read and write as proxy."
            );
            console.log("                       Alternatively, run the following curl request.");
            console.log("```");
            uint256 chainId = block.chainid;
            if (chainId == 1) {
                console.log("curl --request POST 'https://api.etherscan.io/api' \\");
            } else {
                console.log(
                    string.concat(
                        "curl --request POST 'https://api-", vm.getChain(chainId).chainAlias, ".etherscan.io/api' \\"
                    )
                );
            }
            console.log("   --header 'Content-Type: application/x-www-form-urlencoded' \\");
            console.log("   --data-urlencode 'module=contract' \\");
            console.log("   --data-urlencode 'action=verifyproxycontract' \\");
            console.log(string.concat("   --data-urlencode 'address=", vm.toString(contractAddress), "' \\"));
            console.log(
                string.concat(
                    "   --data-urlencode 'expectedimplementation=", vm.toString(expectedImplementation), "' \\"
                )
            );
            console.log("   --data-urlencode \"apikey=$ETHERSCAN_API_KEY\"");
            console.log("```");
        }
        console.log("================================================================================================");
        console.log();
    }
}
