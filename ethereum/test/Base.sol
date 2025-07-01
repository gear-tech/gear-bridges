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
import {CircleToken} from "src/erc20/CircleToken.sol";
import {TetherToken} from "src/erc20/TetherToken.sol";
import {WrappedEther} from "src/erc20/WrappedEther.sol";
import {WrappedVara} from "src/erc20/WrappedVara.sol";
import {IERC20Manager} from "src/interfaces/IERC20Manager.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";
import {VerifierMock} from "src/mocks/VerifierMock.sol";
import {BridgingPayment} from "src/BridgingPayment.sol";
import {ERC20Manager} from "src/ERC20Manager.sol";
import {GovernanceAdmin} from "src/GovernanceAdmin.sol";
import {GovernancePauser} from "src/GovernancePauser.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {Relayer} from "src/Relayer.sol";
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
    uint256 bridgingPaymentFee;
}

library BaseConstants {
    address internal constant ZERO_ADDRESS = address(0);
    uint256 internal constant DEPLOYER_INITIAL_BALANCE = 100 ether;
    address internal constant DEPLOYER_ADDRESS = 0x1111111111111111111111111111111111111111;
    bytes32 internal constant VFT_MANAGER = 0x2222222222222222222222222222222222222222222222222222222222222222;
    bytes32 internal constant GOVERNANCE_ADMIN = 0x3333333333333333333333333333333333333333333333333333333333333333;
    bytes32 internal constant GOVERNANCE_PAUSER = 0x4444444444444444444444444444444444444444444444444444444444444444;
    uint256 internal constant BRIDGING_PAYMENT_FEE = 1 wei;
}

abstract contract Base is CommonBase, StdAssertions, StdChains, StdCheats, StdInvariant, StdUtils {
    DeploymentArguments public deploymentArguments;

    IERC20Metadata public circleToken;
    IERC20Metadata public tetherToken;
    IERC20Metadata public wrappedEther;
    IERC20Metadata public wrappedVara;

    GovernanceAdmin public governanceAdmin;
    GovernancePauser public governancePauser;

    IVerifier public verifier;
    Relayer public relayer;
    MessageQueue public messageQueue;

    ERC20Manager public erc20Manager;

    BridgingPayment public bridgingPayment;

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
            deploymentArguments.deployerAddress, vm.getNonce(deploymentArguments.deployerAddress) + 9
        );

        wrappedVara = new WrappedVara(erc20ManagerAddress);
        console.log("    WVARA:               ", address(wrappedVara));

        console.log();

        //////////////////////////////////////////////////////////////////////////////

        console.log("Bridge governance:");

        address relayerAddress = vm.computeCreateAddress(
            deploymentArguments.deployerAddress, vm.getNonce(deploymentArguments.deployerAddress) + 4
        );
        address messageQueueAddress = vm.computeCreateAddress(
            deploymentArguments.deployerAddress, vm.getNonce(deploymentArguments.deployerAddress) + 6
        );

        address[] memory proxies = new address[](3);

        proxies[0] = relayerAddress;
        proxies[1] = messageQueueAddress;
        proxies[2] = erc20ManagerAddress;

        governanceAdmin = new GovernanceAdmin(deploymentArguments.governanceAdmin, messageQueueAddress, proxies);
        console.log("    GovernanceAdmin:     ", address(governanceAdmin));

        governancePauser = new GovernancePauser(deploymentArguments.governancePauser, messageQueueAddress, proxies);
        console.log("    GovernancePauser:    ", address(governancePauser));

        console.log();

        //////////////////////////////////////////////////////////////////////////////

        console.log("Bridge core:");

        if (isTest) {
            verifier = new VerifierMock();
        } else if (isScript) {
            verifier = new Verifier();
        }

        console.log("    Verifier:            ", address(verifier));

        // TODO: `npm warn exec The following package was not found and will be installed: @openzeppelin/upgrades-core@x.y.z`
        relayer = Relayer(
            Upgrades.deployUUPSProxy("Relayer.sol", abi.encodeCall(Relayer.initialize, (governanceAdmin, verifier)))
        );
        console.log("    Relayer:             ", address(relayer));

        assertEq(relayerAddress, address(relayer));

        messageQueue = MessageQueue(
            Upgrades.deployUUPSProxy(
                "MessageQueue.sol",
                abi.encodeCall(MessageQueue.initialize, (governanceAdmin, governancePauser, relayer))
            )
        );
        console.log("    MessageQueue:        ", address(messageQueue));

        assertEq(messageQueueAddress, address(messageQueue));

        console.log();

        //////////////////////////////////////////////////////////////////////////////

        console.log("Bridge:");

        IERC20Manager.TokenWithSupplyType[] memory tokens = new IERC20Manager.TokenWithSupplyType[](4);

        tokens[0] = IERC20Manager.TokenWithSupplyType(address(circleToken), IERC20Manager.SupplyType.Ethereum);
        tokens[1] = IERC20Manager.TokenWithSupplyType(address(tetherToken), IERC20Manager.SupplyType.Ethereum);
        tokens[2] = IERC20Manager.TokenWithSupplyType(address(wrappedEther), IERC20Manager.SupplyType.Ethereum);
        tokens[3] = IERC20Manager.TokenWithSupplyType(address(wrappedVara), IERC20Manager.SupplyType.Gear);

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

        assertEq(erc20ManagerAddress, address(erc20Manager));

        console.log();

        //////////////////////////////////////////////////////////////////////////////

        console.log("Bridging payment:");

        bridgingPayment = BridgingPayment(erc20Manager.createBridgingPayment(deploymentArguments.bridgingPaymentFee));
        console.log("    BridgingPayment:     ", address(bridgingPayment));

        if (isScript) {
            console.log();

            printContractInfo("Relayer", address(relayer), Upgrades.getImplementationAddress(address(relayer)));
            printContractInfo(
                "MessageQueue", address(messageQueue), Upgrades.getImplementationAddress(address(messageQueue))
            );
            printContractInfo(
                "ERC20Manager", address(erc20Manager), Upgrades.getImplementationAddress(address(erc20Manager))
            );
        }

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
