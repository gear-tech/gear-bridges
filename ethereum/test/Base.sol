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
import {ICircleToken} from "src/erc20/interfaces/ICircleToken.sol";
import {ERC20GearSupply} from "src/erc20/managed/ERC20GearSupply.sol";
import {CircleToken} from "src/erc20/CircleToken.sol";
import {TetherToken} from "src/erc20/TetherToken.sol";
import {WrappedBitcoin} from "src/erc20/WrappedBitcoin.sol";
import {WrappedEther} from "src/erc20/WrappedEther.sol";
import {WrappedVara} from "src/erc20/WrappedVara.sol";
import {IERC20Manager} from "src/interfaces/IERC20Manager.sol";
import {IGovernance} from "src/interfaces/IGovernance.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";
import {MessageHandlerMock} from "src/mocks/MessageHandlerMock.sol";
import {NewImplementationMock} from "src/mocks/NewImplementationMock.sol";
import {VerifierMock} from "src/mocks/VerifierMock.sol";
import {BridgingPayment} from "src/BridgingPayment.sol";
import {ERC20Manager} from "src/ERC20Manager.sol";
import {GovernanceAdmin} from "src/GovernanceAdmin.sol";
import {GovernancePauser} from "src/GovernancePauser.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {VerifierMainnet} from "src/VerifierMainnet.sol";
import {VerifierTestnet} from "src/VerifierTestnet.sol";

import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";
import {
    PauseProxyMessage,
    UnpauseProxyMessage,
    UpgradeProxyMessage,
    GovernancePacker
} from "src/interfaces/IGovernance.sol";
import {
    IERC20Manager,
    TransferMessage,
    AddVftManagerMessage,
    RegisterEthereumTokenMessage,
    RegisterGearTokenMessage,
    ERC20ManagerPacker
} from "src/interfaces/IERC20Manager.sol";
import {VaraMessage, IMessageQueue, Hasher} from "src/interfaces/IMessageQueue.sol";

struct Overrides {
    address circleToken;
    address tetherToken;
    address wrappedEther;
    address wrappedBitcoin;
}

struct DeploymentArguments {
    uint256 privateKey;
    address deployerAddress;
    string forkUrlOrAlias;
    Overrides overrides;
    bytes32 vftManager;
    bytes32 governanceAdmin;
    bytes32 governancePauser;
    address emergencyStopAdmin;
    address[] emergencyStopObservers;
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
    address internal constant EMERGENCY_STOP_OBSERVER1 = 0x6666666666666666666666666666666666666666;
    address internal constant EMERGENCY_STOP_OBSERVER2 = 0x7777777777777777777777777777777777777777;
    uint256 internal constant BRIDGING_PAYMENT_FEE = 1 wei;
}

abstract contract Base is CommonBase, StdAssertions, StdChains, StdCheats, StdInvariant, StdUtils {
    using Hasher for VaraMessage;

    using GovernancePacker for PauseProxyMessage;
    using GovernancePacker for UnpauseProxyMessage;
    using GovernancePacker for UpgradeProxyMessage;

    using ERC20ManagerPacker for TransferMessage;
    using ERC20ManagerPacker for AddVftManagerMessage;
    using ERC20ManagerPacker for RegisterEthereumTokenMessage;
    using ERC20ManagerPacker for RegisterGearTokenMessage;

    uint256 public messageNonce;
    uint256 public currentBlockNumber;

    DeploymentArguments public deploymentArguments;

    IERC20Metadata public erc20GearSupply;

    IERC20Metadata public circleToken;
    IERC20Metadata public tetherToken;
    IERC20Metadata public wrappedEther;
    IERC20Metadata public wrappedBitcoin;

    WrappedVara public wrappedVara;

    GovernanceAdmin public governanceAdmin;
    GovernancePauser public governancePauser;

    IVerifier public verifier;
    MessageQueue public messageQueue;

    ERC20Manager public erc20Manager;

    BridgingPayment public bridgingPayment;

    MessageHandlerMock public messageHandlerMock;
    NewImplementationMock public newImplementationMock;

    function deployBridgeDependsOnEnvironment() public {
        if (vm.envExists("FORK_URL_OR_ALIAS")) {
            deployBridgeFromExistingNetwork();
        } else {
            deployBridgeFromConstants();
        }
    }

    function deployBridgeFromConstants() public {
        deployBridgeFromConstants(BaseConstants.DEPLOYER_ADDRESS, "");
    }

    function deployBridgeFromExistingNetwork() public {
        address deployerAddress = vm.envAddress("DEPLOYER_ADDRESS");
        string memory forkUrlOrAlias = vm.envString("FORK_URL_OR_ALIAS");

        deployBridgeFromConstants(deployerAddress, forkUrlOrAlias);
    }

    function deployBridgeFromConstants(address deployerAddress, string memory forkUrlOrAlias) public {
        address[] memory emergencyStopObservers = new address[](2);

        emergencyStopObservers[0] = BaseConstants.EMERGENCY_STOP_OBSERVER1;
        emergencyStopObservers[1] = BaseConstants.EMERGENCY_STOP_OBSERVER2;

        deployBridge(
            DeploymentArguments({
                privateKey: 0,
                deployerAddress: deployerAddress,
                forkUrlOrAlias: forkUrlOrAlias,
                overrides: Overrides({
                    circleToken: BaseConstants.ZERO_ADDRESS,
                    tetherToken: BaseConstants.ZERO_ADDRESS,
                    wrappedEther: BaseConstants.ZERO_ADDRESS,
                    wrappedBitcoin: BaseConstants.ZERO_ADDRESS
                }),
                vftManager: BaseConstants.VFT_MANAGER,
                governanceAdmin: BaseConstants.GOVERNANCE_ADMIN,
                governancePauser: BaseConstants.GOVERNANCE_PAUSER,
                emergencyStopAdmin: BaseConstants.EMERGENCY_STOP_ADMIN,
                emergencyStopObservers: emergencyStopObservers,
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
                forkUrlOrAlias: "",
                overrides: Overrides({
                    circleToken: vm.envExists("CIRCLE_TOKEN")
                        ? vm.envAddress("CIRCLE_TOKEN")
                        : BaseConstants.ZERO_ADDRESS,
                    tetherToken: vm.envExists("TETHER_TOKEN")
                        ? vm.envAddress("TETHER_TOKEN")
                        : BaseConstants.ZERO_ADDRESS,
                    wrappedEther: vm.envExists("WRAPPED_ETHER")
                        ? vm.envAddress("WRAPPED_ETHER")
                        : BaseConstants.ZERO_ADDRESS,
                    wrappedBitcoin: vm.envExists("WRAPPED_BITCOIN")
                        ? vm.envAddress("WRAPPED_BITCOIN")
                        : BaseConstants.ZERO_ADDRESS
                }),
                vftManager: vm.envBytes32("VFT_MANAGER"),
                governanceAdmin: vm.envBytes32("GOVERNANCE_ADMIN"),
                governancePauser: vm.envBytes32("GOVERNANCE_PAUSER"),
                emergencyStopAdmin: vm.envAddress("EMERGENCY_STOP_ADMIN"),
                emergencyStopObservers: vm.envAddress("EMERGENCY_STOP_OBSERVERS", ","),
                bridgingPaymentFee: vm.envUint("BRIDGING_PAYMENT_FEE")
            })
        );
    }

    function deployBridge(DeploymentArguments memory _deploymentArguments) public {
        deploymentArguments = _deploymentArguments;

        bool isTest = deploymentArguments.privateKey == 0;
        bool isScript = !isTest;
        bool isFork = bytes(deploymentArguments.forkUrlOrAlias).length != 0;

        if (isFork) {
            console.log(string.concat("Forking on ", deploymentArguments.forkUrlOrAlias, "..."));

            console.log();

            vm.createSelectFork(deploymentArguments.forkUrlOrAlias);

            governanceAdmin = GovernanceAdmin(vm.envAddress("GOVERNANCE_ADMIN_CONTRACT"));
            governancePauser = GovernancePauser(vm.envAddress("GOVERNANCE_PAUSER_CONTRACT"));

            wrappedVara = WrappedVara(governanceAdmin.wrappedVara());
            messageQueue = MessageQueue(governanceAdmin.messageQueue());
            erc20Manager = ERC20Manager(governanceAdmin.erc20Manager());

            verifier = IVerifier(messageQueue.verifier());
            vm.etch(address(verifier), type(VerifierMock).runtimeCode);
            VerifierMock(address(verifier)).setValue(true);

            messageNonce = 100_000_000;
            currentBlockNumber = messageQueue.maxBlockNumber() + 1;

            address[] memory erc20Tokens = erc20Manager.tokens(0, 5);
            bridgingPayment = BridgingPayment(erc20Manager.bridgingPayments()[0]);

            Overrides memory overrides = Overrides({
                circleToken: erc20Tokens[0],
                tetherToken: erc20Tokens[1],
                wrappedEther: erc20Tokens[2],
                wrappedBitcoin: erc20Tokens[4]
            });

            bytes32 slot = bytes32(uint256(0x08)); // address masterMinter
            bytes32 value = ((vm.load(address(overrides.circleToken), slot) >> 160) << 160)
                | bytes32(uint256(uint160(deploymentArguments.deployerAddress)));
            vm.store(address(overrides.circleToken), slot, value);

            vm.prank(deploymentArguments.deployerAddress);
            ICircleToken(address(overrides.circleToken))
                .configureMinter(deploymentArguments.deployerAddress, type(uint256).max);

            slot = bytes32(0x00); // address owner
            value = bytes32(uint256(uint160(deploymentArguments.deployerAddress)));
            vm.store(overrides.tetherToken, slot, value);

            slot = bytes32(uint256(0x05)); // address owner
            value = ((vm.load(address(overrides.wrappedBitcoin), slot) << 248) >> 248)
                | (bytes32(uint256(uint160(deploymentArguments.deployerAddress))) << 8);
            vm.store(overrides.wrappedBitcoin, slot, value);

            deploymentArguments = DeploymentArguments({
                privateKey: 0,
                deployerAddress: _deploymentArguments.deployerAddress,
                forkUrlOrAlias: _deploymentArguments.forkUrlOrAlias,
                overrides: overrides,
                vftManager: erc20Manager.vftManagers()[0],
                governanceAdmin: governanceAdmin.governance(),
                governancePauser: governancePauser.governance(),
                emergencyStopAdmin: messageQueue.emergencyStopAdmin(),
                emergencyStopObservers: messageQueue.emergencyStopObservers(),
                bridgingPaymentFee: bridgingPayment.fee()
            });

            // TODO: all manipulations with the forked contracts should be done here

            // make message with upgrade of MessageQueue implementation and reinitialization processed
            // https://etherscan.io/tx/0xfae7c0aec41c8d419ec1cd18fe99e60dd30e253efaf8d9e8d3b771146322f080
            address newImplementation = 0x9D5D2BCf93feD81e48CCb645112F008aD6098eE7;

            VaraMessage memory message0 = VaraMessage({
                nonce: type(uint256).max,
                source: governanceAdmin.governance(),
                destination: address(governanceAdmin),
                payload: UpgradeProxyMessage({
                        proxy: address(messageQueue),
                        newImplementation: newImplementation,
                        data: abi.encodeWithSelector(MessageQueue.reinitialize.selector)
                    }).pack()
            });

            bytes32 message0Hash = message0.hash();

            // https://etherscan.io/tx/0xfbf36c984c9c0f65b16e58917111cba6858409364dde25a8a3df711c5e844993
            // test that merkle root at block 28_125_135 was in storage
            assertEq(messageQueue.getMerkleRoot(28_125_135), message0Hash);
            assertEq(messageQueue.getMerkleRootTimestamp(message0Hash), 1764952379);

            // test that message0.nonce was in storage
            assertEq(messageQueue.isProcessed(message0.nonce), true);

            newImplementation = address(new MessageQueue()); // TODO: replace with actual new implementation

            VaraMessage memory message1 = VaraMessage({
                nonce: 146,
                source: governanceAdmin.governance(),
                destination: address(governanceAdmin),
                payload: UpgradeProxyMessage({
                        proxy: address(messageQueue),
                        newImplementation: newImplementation,
                        data: abi.encodeWithSelector(MessageQueue.reinitialize.selector)
                    }).pack()
            });
            assertEq(messageQueue.isProcessed(message1.nonce), false);

            bytes32 messageHash = message1.hash(); // TODO: print messageHash

            uint256 blockNumber = currentBlockNumber++;
            bytes32 merkleRoot = messageHash;
            bytes memory proof1 = "";

            vm.expectEmit(address(messageQueue));
            emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

            messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

            vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_ADMIN_MESSAGE_DELAY());

            uint256 totalLeaves = 1;
            uint256 leafIndex = 0;
            bytes32[] memory proof2 = new bytes32[](0);

            messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message1, proof2);
            assertEq(
                address(uint160(uint256(vm.load(address(messageQueue), ERC1967Utils.IMPLEMENTATION_SLOT)))),
                address(newImplementation)
            );

            // test that merkle root at block 28_125_135 was removed from storage
            assertEq(messageQueue.getMerkleRoot(28_125_135), bytes32(0));
            assertEq(messageQueue.getMerkleRootTimestamp(message0Hash), 0);

            // test that type(uint256).max nonce was removed from storage
            assertEq(messageQueue.isProcessed(type(uint256).max), false);

            // test processing of all paid messages on fork

            // message #141

            VaraMessage memory message2 = VaraMessage({
                nonce: 141,
                source: deploymentArguments.vftManager,
                destination: address(erc20Manager),
                payload: TransferMessage({
                        sender: 0xaaa85e844473d825a1645513ed1cafe2669f31d98d187579f70d114f017df84a,
                        receiver: 0x61d052FbDF5a0Cfff9D1C27ff4E2034Ba9f29396,
                        token: address(wrappedVara),
                        amount: 1_000 * (10 ** wrappedVara.decimals())
                    }).pack()
            });
            assertEq(messageQueue.isProcessed(message2.nonce), false);

            messageHash = message2.hash();
            assertEq(messageHash, 0x35a87dc240b786879dbe9705045c3140abea49cc0b1a14dbca768bcb26f08f88); // https://vara.subscan.io/event/28135906-30

            blockNumber = currentBlockNumber++;
            merkleRoot = messageHash;

            vm.expectEmit(address(messageQueue));
            emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

            messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

            vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

            totalLeaves = 1;
            leafIndex = 0;
            proof2 = new bytes32[](0);

            messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message2, proof2);

            // https://etherscan.io/address/0x61d052FbDF5a0Cfff9D1C27ff4E2034Ba9f29396
            assertEq(
                wrappedVara.balanceOf(0x61d052FbDF5a0Cfff9D1C27ff4E2034Ba9f29396),
                (126_000 + 1_000) * (10 ** wrappedVara.decimals())
            );

            // message #142

            VaraMessage memory message3 = VaraMessage({
                nonce: 142,
                source: deploymentArguments.vftManager,
                destination: address(erc20Manager),
                payload: TransferMessage({
                        sender: 0x50bf690723c14c7242b3eb0488f4ceb26e140f79fd4af16551048976ed119433,
                        receiver: 0xaB012236A482fB9bFA83b955EC7b6115A0D8f714,
                        token: address(wrappedVara),
                        amount: 300_200 * (10 ** wrappedVara.decimals())
                    }).pack()
            });
            assertEq(messageQueue.isProcessed(message3.nonce), false);

            messageHash = message3.hash();
            assertEq(messageHash, 0xb73ff7404e14b4e13d5e6038bac20b9c5e5d6d269879841b61a881da6d0a35f3); // https://vara.subscan.io/event/28141439-31

            blockNumber = currentBlockNumber++;
            merkleRoot = 0x969589f86884f4399a51b940a58d728b04e12fc30f21c1fddce8ca0eb82f9734;

            vm.expectEmit(address(messageQueue));
            emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

            messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

            vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

            totalLeaves = 4;
            leafIndex = 0;
            proof2 = new bytes32[](2);
            proof2[0] = 0xc2bb624fb7b5f2e5fe29927ebd1b3837548aa1ecb3379242bd7c2f0666f39668;
            proof2[1] = 0x609cf669aae21aef2be635fadde5e5aab418d0e656e4900d898b94f335e83007;

            messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message3, proof2);

            // https://etherscan.io/address/0xaB012236A482fB9bFA83b955EC7b6115A0D8f714
            assertEq(
                wrappedVara.balanceOf(0xaB012236A482fB9bFA83b955EC7b6115A0D8f714),
                (10 + 300_200) * (10 ** wrappedVara.decimals())
            );

            // message #144

            VaraMessage memory message4 = VaraMessage({
                nonce: 144,
                source: deploymentArguments.vftManager,
                destination: address(erc20Manager),
                payload: TransferMessage({
                        sender: 0x50bf690723c14c7242b3eb0488f4ceb26e140f79fd4af16551048976ed119433,
                        receiver: 0xaB012236A482fB9bFA83b955EC7b6115A0D8f714,
                        token: address(wrappedVara),
                        amount: 1_000 * (10 ** wrappedVara.decimals())
                    }).pack()
            });
            assertEq(messageQueue.isProcessed(message4.nonce), false);

            messageHash = message4.hash();
            assertEq(messageHash, 0x3ba6a14f187cd7ac3956ed4f302b559938c9ecdd44066b95fbfabe3b43942689); // https://vara.subscan.io/event/28142735-30

            blockNumber = currentBlockNumber++;
            merkleRoot = 0x969589f86884f4399a51b940a58d728b04e12fc30f21c1fddce8ca0eb82f9734;

            vm.expectEmit(address(messageQueue));
            emit IMessageQueue.MerkleRoot(blockNumber, merkleRoot);

            messageQueue.submitMerkleRoot(blockNumber, merkleRoot, proof1);

            vm.warp(vm.getBlockTimestamp() + messageQueue.PROCESS_USER_MESSAGE_DELAY());

            totalLeaves = 4;
            leafIndex = 2;
            proof2 = new bytes32[](2);
            proof2[0] = 0x2d6efc7a1d195950d5e0606da04837b4c9954a163c7fff5188d312f78b776d7e;
            proof2[1] = 0x6d5d607e43e359f7f78175da3aba38fb7708859c7a48f9aaf1704ea7eb8cb3c9;

            messageQueue.processMessage(blockNumber, totalLeaves, leafIndex, message4, proof2);

            // https://etherscan.io/address/0xaB012236A482fB9bFA83b955EC7b6115A0D8f714
            assertEq(
                wrappedVara.balanceOf(0xaB012236A482fB9bFA83b955EC7b6115A0D8f714),
                (10 + 300_200 + 1_000) * (10 ** wrappedVara.decimals())
            );
        }

        console.log("Deployment arguments:");

        console.log("    deployerAddress:     ", deploymentArguments.deployerAddress);
        console.log("    vftManager:          ", vm.toString(deploymentArguments.vftManager));
        console.log("    governanceAdmin:     ", vm.toString(deploymentArguments.governanceAdmin));
        console.log("    governancePauser:    ", vm.toString(deploymentArguments.governancePauser));
        console.log("    bridgingPaymentFee:  ", deploymentArguments.bridgingPaymentFee, "wei");

        if (isTest) {
            if (!isFork) {
                vm.warp(vm.unixTime() / 1000);
            }
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

        if (isTest && !isFork) {
            deployTestTokens();
        } else if (isScript || isFork) {
            if (shouldUseOverrides()) {
                circleToken = IERC20Metadata(deploymentArguments.overrides.circleToken);
                tetherToken = IERC20Metadata(deploymentArguments.overrides.tetherToken);
                wrappedEther = IERC20Metadata(deploymentArguments.overrides.wrappedEther);
                wrappedBitcoin = IERC20Metadata(deploymentArguments.overrides.wrappedBitcoin);
            } else {
                deployTestTokens();
            }
        }

        console.log("    USDC:                ", address(circleToken));
        console.log("    USDT:                ", address(tetherToken));
        console.log("    WETH:                ", address(wrappedEther));
        console.log("    WBTC:                ", address(wrappedBitcoin));

        address erc20ManagerAddress = vm.computeCreateAddress(
            deploymentArguments.deployerAddress, vm.getNonce(deploymentArguments.deployerAddress) + 8
        );
        address governanceAdminAddress = vm.computeCreateAddress(
            deploymentArguments.deployerAddress, vm.getNonce(deploymentArguments.deployerAddress) + 2
        );
        address governancePauserAddress = vm.computeCreateAddress(
            deploymentArguments.deployerAddress, vm.getNonce(deploymentArguments.deployerAddress) + 3
        );

        // TODO: `npm warn exec The following package was not found and will be installed: @openzeppelin/upgrades-core@x.y.z`
        if (!isFork) {
            wrappedVara = WrappedVara(
                Upgrades.deployUUPSProxy(
                    "WrappedVara.sol",
                    abi.encodeCall(
                        WrappedVara.initialize,
                        (IGovernance(governanceAdminAddress), IGovernance(governancePauserAddress), erc20ManagerAddress)
                    )
                )
            );
        }

        uint256 chainId = block.chainid;

        if (chainId == 1) {
            console.log("    WVARA:               ", address(wrappedVara));
        } else {
            console.log("    WTVARA:              ", address(wrappedVara));
        }

        if (!isFork) {
            assertEq(wrappedVara.governanceAdmin(), governanceAdminAddress);
            assertEq(wrappedVara.governancePauser(), governancePauserAddress);
            assertEq(wrappedVara.minter(), erc20ManagerAddress);
        } else {
            assertEq(wrappedVara.governanceAdmin(), address(governanceAdmin));
            assertEq(wrappedVara.governancePauser(), address(governancePauser));
            assertEq(wrappedVara.minter(), address(erc20Manager));
        }

        console.log();

        //////////////////////////////////////////////////////////////////////////////

        console.log("Bridge governance:");

        address messageQueueAddress = vm.computeCreateAddress(
            deploymentArguments.deployerAddress, vm.getNonce(deploymentArguments.deployerAddress) + 4
        );

        if (!isFork) {
            governanceAdmin = new GovernanceAdmin(
                deploymentArguments.governanceAdmin, address(wrappedVara), messageQueueAddress, erc20ManagerAddress
            );
        }
        console.log("    GovernanceAdmin:     ", address(governanceAdmin));

        if (!isFork) {
            assertEq(governanceAdminAddress, address(governanceAdmin));
        }

        if (!isFork) {
            governancePauser = new GovernancePauser(
                deploymentArguments.governancePauser, address(wrappedVara), messageQueueAddress, erc20ManagerAddress
            );
        }
        console.log("    GovernancePauser:    ", address(governancePauser));

        if (!isFork) {
            assertEq(governancePauserAddress, address(governancePauser));
        }

        console.log();

        //////////////////////////////////////////////////////////////////////////////

        console.log("Bridge core:");

        if (!isFork) {
            if (isTest) {
                verifier = new VerifierMock(true);
            } else if (isScript) {
                if (chainId == 1) {
                    verifier = new VerifierMainnet();
                } else {
                    verifier = new VerifierTestnet();
                }
            }
        }

        console.log("    Verifier:            ", address(verifier));

        // TODO: `npm warn exec The following package was not found and will be installed: @openzeppelin/upgrades-core@x.y.z`
        if (!isFork) {
            messageQueue = MessageQueue(
                Upgrades.deployUUPSProxy(
                    "MessageQueue.sol",
                    abi.encodeCall(
                        MessageQueue.initialize,
                        (
                            governanceAdmin,
                            governancePauser,
                            deploymentArguments.emergencyStopAdmin,
                            deploymentArguments.emergencyStopObservers,
                            verifier
                        )
                    )
                )
            );
        }
        console.log("    MessageQueue:        ", address(messageQueue));

        messageQueueAssertions(isFork ? address(messageQueue) : messageQueueAddress);

        console.log();

        //////////////////////////////////////////////////////////////////////////////

        console.log("Bridge:");

        if (!isFork) {
            IERC20Manager.TokenInfo[] memory tokens = new IERC20Manager.TokenInfo[](5);

            tokens[0] = IERC20Manager.TokenInfo(address(circleToken), IERC20Manager.TokenType.Ethereum);
            tokens[1] = IERC20Manager.TokenInfo(address(tetherToken), IERC20Manager.TokenType.Ethereum);
            tokens[2] = IERC20Manager.TokenInfo(address(wrappedEther), IERC20Manager.TokenType.Ethereum);
            tokens[3] = IERC20Manager.TokenInfo(address(wrappedVara), IERC20Manager.TokenType.Gear);
            tokens[4] = IERC20Manager.TokenInfo(address(wrappedBitcoin), IERC20Manager.TokenType.Ethereum);

            erc20Manager = ERC20Manager(
                Upgrades.deployUUPSProxy(
                    "ERC20Manager.sol",
                    abi.encodeCall(
                        ERC20Manager.initialize,
                        (
                            governanceAdmin,
                            governancePauser,
                            address(messageQueue),
                            deploymentArguments.vftManager,
                            tokens
                        )
                    )
                )
            );
        }
        console.log("    ERC20Manager:        ", address(erc20Manager));

        erc20ManagerAssertions(isFork ? address(erc20Manager) : erc20ManagerAddress);

        //////////////////////////////////////////////////////////////////////////////

        console.log("Bridging payment:");

        if (!isFork) {
            bridgingPayment =
                BridgingPayment(erc20Manager.createBridgingPayment(deploymentArguments.bridgingPaymentFee));
        }
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
                "WrappedVara", address(wrappedVara), Upgrades.getImplementationAddress(address(wrappedVara))
            );
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
        wrappedBitcoin = new WrappedBitcoin(deploymentArguments.deployerAddress);
    }

    function shouldUseOverrides() public view returns (bool) {
        return deploymentArguments.overrides.circleToken != BaseConstants.ZERO_ADDRESS
            && deploymentArguments.overrides.tetherToken != BaseConstants.ZERO_ADDRESS
            && deploymentArguments.overrides.wrappedEther != BaseConstants.ZERO_ADDRESS
            && deploymentArguments.overrides.wrappedBitcoin != BaseConstants.ZERO_ADDRESS;
    }

    function messageQueueAssertions(address messageQueueAddress) public view {
        assertEq(messageQueueAddress, address(messageQueue));
        assertEq(messageQueue.governanceAdmin(), address(governanceAdmin));
        assertEq(messageQueue.governancePauser(), address(governancePauser));
        assertEq(messageQueue.emergencyStopAdmin(), deploymentArguments.emergencyStopAdmin);
        address[] memory emergencyStopObservers = messageQueue.emergencyStopObservers();
        assertEq(emergencyStopObservers.length, deploymentArguments.emergencyStopObservers.length);
        for (uint256 i = 0; i < emergencyStopObservers.length; i++) {
            assertEq(emergencyStopObservers[i], deploymentArguments.emergencyStopObservers[i]);
        }
        assertEq(messageQueue.verifier(), address(verifier));
        assertEq(messageQueue.isChallengingRoot(), false);
        assertEq(messageQueue.isEmergencyStopped(), false);
        if (!isFork()) {
            assertEq(messageQueue.genesisBlock(), 0);
            assertEq(messageQueue.maxBlockNumber(), 0);
        }
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
        assertEq(erc20Manager.totalTokens(), 5);
        address[] memory tokens1 = erc20Manager.tokens();
        assertEq(tokens1.length, 5);
        assertEq(tokens1[0], address(circleToken));
        assertEq(tokens1[1], address(tetherToken));
        assertEq(tokens1[2], address(wrappedEther));
        assertEq(tokens1[3], address(wrappedVara));
        assertEq(tokens1[4], address(wrappedBitcoin));
        address[] memory tokens2 = erc20Manager.tokens(5, 5);
        assertEq(tokens2.length, 0);
        address[] memory tokens3 = erc20Manager.tokens(0, 2);
        assertEq(tokens3.length, 2);
        assertEq(tokens3[0], address(circleToken));
        assertEq(tokens3[1], address(tetherToken));
        address[] memory tokens4 = erc20Manager.tokens(2, 3);
        assertEq(tokens4.length, 3);
        assertEq(tokens4[0], address(wrappedEther));
        assertEq(tokens4[1], address(wrappedVara));
        assertEq(tokens4[2], address(wrappedBitcoin));
        address[] memory tokens5 = erc20Manager.tokens(0, 6);
        assertEq(tokens5.length, 5);
        assertEq(tokens5[0], address(circleToken));
        assertEq(tokens5[1], address(tetherToken));
        assertEq(tokens5[2], address(wrappedEther));
        assertEq(tokens5[3], address(wrappedVara));
        assertEq(tokens5[4], address(wrappedBitcoin));
        assertTrue(erc20Manager.getTokenType(address(circleToken)) == IERC20Manager.TokenType.Ethereum);
        assertTrue(erc20Manager.getTokenType(address(tetherToken)) == IERC20Manager.TokenType.Ethereum);
        assertTrue(erc20Manager.getTokenType(address(wrappedEther)) == IERC20Manager.TokenType.Ethereum);
        assertTrue(erc20Manager.getTokenType(address(wrappedVara)) == IERC20Manager.TokenType.Gear);
        assertTrue(erc20Manager.getTokenType(address(wrappedBitcoin)) == IERC20Manager.TokenType.Ethereum);
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
            console.log("curl \\");
            console.log(string.concat("    --data \"address=", vm.toString(contractAddress), "\" \\"));
            console.log(
                string.concat("    --data \"expectedimplementation=", vm.toString(expectedImplementation), "\" \\")
            );
            console.log(
                string.concat(
                    "    \"https://api.etherscan.io/v2/api?chainid=",
                    vm.toString(chainId),
                    "&module=contract&action=verifyproxycontract&apikey=$ETHERSCAN_API_KEY\""
                )
            );
            console.log("```");
        }
        console.log("================================================================================================");
        console.log();
    }
}
