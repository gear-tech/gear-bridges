// Copyright (C) Gear Technologies Inc.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {IERC20Metadata} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Metadata.sol";
import {CommonBase} from "forge-std/Base.sol";
import {StdAssertions} from "forge-std/StdAssertions.sol";
import {StdChains} from "forge-std/StdChains.sol";
import {StdCheats} from "forge-std/StdCheats.sol";
import {StdInvariant} from "forge-std/StdInvariant.sol";
import {StdUtils} from "forge-std/StdUtils.sol";
import {console} from "forge-std/console.sol";
import {Upgrades} from "openzeppelin-foundry-upgrades/Upgrades.sol";
import {BridgingPayment} from "src/BridgingPayment.sol";
import {ERC20Manager} from "src/ERC20Manager.sol";
import {GovernanceAdmin} from "src/GovernanceAdmin.sol";
import {GovernancePauser} from "src/GovernancePauser.sol";
import {MessageQueue} from "src/MessageQueue.sol";
import {IRecoveryThresholdWallet} from "src/RecoveryController.sol";
import {VerifierMainnet} from "src/VerifierMainnet.sol";
import {VerifierTestnet} from "src/VerifierTestnet.sol";
import {CircleToken} from "src/erc20/CircleToken.sol";
import {TetherToken} from "src/erc20/TetherToken.sol";
import {WrappedBitcoin} from "src/erc20/WrappedBitcoin.sol";
import {WrappedEther} from "src/erc20/WrappedEther.sol";
import {WrappedVara} from "src/erc20/WrappedVara.sol";
import {ICircleToken} from "src/erc20/interfaces/ICircleToken.sol";
import {ERC20GearSupply} from "src/erc20/managed/ERC20GearSupply.sol";
import {IERC20Manager} from "src/interfaces/IERC20Manager.sol";
import {IGovernance} from "src/interfaces/IGovernance.sol";
import {IVerifier} from "src/interfaces/IVerifier.sol";
import {MessageHandlerMock} from "src/mocks/MessageHandlerMock.sol";
import {NewImplementationMock} from "src/mocks/NewImplementationMock.sol";
import {VerifierMock} from "src/mocks/VerifierMock.sol";
import {BaseConstants} from "test/BaseConstants.sol";

import {IERC1967} from "@openzeppelin/contracts/interfaces/IERC1967.sol";
import {ERC1967Utils} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Utils.sol";
import {IMessageQueue, VaraMessage} from "src/interfaces/IMessageQueue.sol";
import {Hasher} from "src/libraries/Hasher.sol";
import {ERC20ManagerPacker, TransferMessage} from "src/libraries/packing/ERC20ManagerPacker.sol";
import {GovernancePacker, UpgradeProxyMessage} from "src/libraries/packing/GovernancePacker.sol";

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
    address recoveryWallet;
}

abstract contract Base is CommonBase, StdAssertions, StdChains, StdCheats, StdInvariant, StdUtils {
    using Hasher for VaraMessage;

    using GovernancePacker for UpgradeProxyMessage;

    using ERC20ManagerPacker for TransferMessage;

    uint256 public messageNonce;
    uint256 public currentBlockNumber = 1;

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
    address internal expectedMessageQueueAddress;
    bool internal includeOrdinaryGearToken;

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
                bridgingPaymentFee: BaseConstants.BRIDGING_PAYMENT_FEE,
                recoveryWallet: address(0)
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
                bridgingPaymentFee: vm.envUint("BRIDGING_PAYMENT_FEE"),
                recoveryWallet: address(0)
            })
        );
    }

    function _validateRecoveryWallet(address recoveryWallet) internal view {
        require(recoveryWallet != address(0) && recoveryWallet.code.length != 0, "missing deployed recovery Safe");
        IRecoveryThresholdWallet wallet = IRecoveryThresholdWallet(recoveryWallet);
        address[] memory owners = wallet.getOwners();
        require(wallet.getThreshold() == 3 && owners.length == 5, "recovery Safe must be 3-of-5");
    }

    /// forge-lint: disable-next-item(cyclomatic-complexity)
    function deployBridge(DeploymentArguments memory _deploymentArguments) public {
        deploymentArguments = _deploymentArguments;

        bool isTest = deploymentArguments.privateKey == 0;
        bool isScript = !isTest;
        bool isFork = bytes(deploymentArguments.forkUrlOrAlias).length != 0;

        if (isFork) {
            console.log(string.concat("Forking on ", deploymentArguments.forkUrlOrAlias, "..."));

            console.log();

            // forge-lint: disable-next-item(reentrancy-no-eth, unused-return)
            vm.createSelectFork(deploymentArguments.forkUrlOrAlias);

            governanceAdmin = GovernanceAdmin(vm.envAddress("GOVERNANCE_ADMIN_CONTRACT"));
            governancePauser = GovernancePauser(vm.envAddress("GOVERNANCE_PAUSER_CONTRACT"));

            wrappedVara = governanceAdmin.wrappedVara();
            messageQueue = governanceAdmin.messageQueue();
            erc20Manager = governanceAdmin.erc20Manager();

            verifier = IVerifier(messageQueue.verifier());
            // forge-lint: disable-next-item(reentrancy-no-eth)
            vm.etch(address(verifier), type(VerifierMock).runtimeCode);
            // forge-lint: disable-next-item(reentrancy-no-eth)
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

            circleToken = IERC20Metadata(overrides.circleToken);
            tetherToken = IERC20Metadata(overrides.tetherToken);
            wrappedEther = IERC20Metadata(overrides.wrappedEther);
            wrappedBitcoin = IERC20Metadata(overrides.wrappedBitcoin);

            bool isMainnet = block.chainid == 1;
            if (isMainnet) {
                bytes32 slot = bytes32(uint256(0x08)); // address masterMinter
                bytes32 value = ((vm.load(address(overrides.circleToken), slot) >> 160) << 160)
                    | bytes32(uint256(uint160(deploymentArguments.deployerAddress)));
                // forge-lint: disable-next-item(reentrancy-no-eth)
                vm.store(address(overrides.circleToken), slot, value);

                // forge-lint: disable-next-item(reentrancy-no-eth)
                vm.prank(deploymentArguments.deployerAddress);
                // forge-lint: disable-next-item(reentrancy-no-eth)
                ICircleToken(address(overrides.circleToken))
                    .configureMinter(deploymentArguments.deployerAddress, type(uint256).max);

                slot = bytes32(0x00); // address owner
                value = bytes32(uint256(uint160(deploymentArguments.deployerAddress)));
                // forge-lint: disable-next-item(reentrancy-no-eth)
                vm.store(overrides.tetherToken, slot, value);

                slot = bytes32(uint256(0x05)); // address owner
                value = ((vm.load(address(overrides.wrappedBitcoin), slot) << 248) >> 248)
                    | (bytes32(uint256(uint160(deploymentArguments.deployerAddress))) << 8);
                // forge-lint: disable-next-item(reentrancy-no-eth)
                vm.store(overrides.wrappedBitcoin, slot, value);
            }

            deploymentArguments = DeploymentArguments({
                privateKey: 0,
                deployerAddress: _deploymentArguments.deployerAddress,
                forkUrlOrAlias: _deploymentArguments.forkUrlOrAlias,
                overrides: overrides,
                vftManager: erc20Manager.vftManagers()[isMainnet ? 2 : 0],
                governanceAdmin: governanceAdmin.governance(),
                governancePauser: governancePauser.governance(),
                emergencyStopAdmin: messageQueue.emergencyStopAdmin(),
                emergencyStopObservers: messageQueue.emergencyStopObservers(),
                bridgingPaymentFee: bridgingPayment.fee(),
                recoveryWallet: _deploymentArguments.recoveryWallet
            });

            if (messageQueue.isChallengingRoot()) {
                // forge-lint: disable-next-item(reentrancy-no-eth)
                vm.prank(deploymentArguments.emergencyStopAdmin);
                // forge-lint: disable-next-item(reentrancy-no-eth)
                messageQueue.disableChallengeRoot();
            }

            // forge-lint: disable-next-line(todo-comment)
            // TODO: all manipulations with the forked contracts should be done here

            address newImplementation1 = address(new MessageQueue());

            VaraMessage memory message1 = VaraMessage({
                nonce: type(uint256).max,
                source: governanceAdmin.governance(),
                destination: address(governanceAdmin),
                payload: UpgradeProxyMessage({
                    proxy: address(messageQueue),
                    newImplementation: newImplementation1,
                    data: abi.encodeWithSelector(MessageQueue.reinitialize.selector)
                }).pack()
            });
            console.logBytes(message1.payload);
            assertEq(messageQueue.isProcessed(message1.nonce), false);

            bytes32 messageHash = message1.hash();
            // assertEq(messageHash, 0x...);

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
                address(newImplementation1)
            );

            // test after upgrade

            address multiSigWallet = 0x1111111111111111111111111111111111111111;
            address newImplementation2 = address(new MessageQueue());

            vm.startPrank(multiSigWallet);

            vm.expectEmit(address(messageQueue));
            // forge-lint: disable-next-item(reentrancy-events)
            emit IERC1967.Upgraded(address(newImplementation2));

            messageQueue.upgradeToAndCall(newImplementation2, new bytes(0));

            vm.stopPrank();
        }

        console.log("Deployment arguments:");

        console.log("    deployerAddress:     ", deploymentArguments.deployerAddress);
        console.log("    vftManager:          ", vm.toString(deploymentArguments.vftManager));
        console.log("    governanceAdmin:     ", vm.toString(deploymentArguments.governanceAdmin));
        console.log("    governancePauser:    ", vm.toString(deploymentArguments.governancePauser));
        console.log("    bridgingPaymentFee:  ", deploymentArguments.bridgingPaymentFee, "wei");

        if (isTest) {
            if (!isFork) {
                // forge-lint: disable-next-item(reentrancy-no-eth)
                vm.warp(vm.unixTime() / 1000);
            }
            // forge-lint: disable-next-item(reentrancy-no-eth)
            vm.deal(deploymentArguments.deployerAddress, BaseConstants.DEPLOYER_INITIAL_BALANCE);
            // forge-lint: disable-next-item(reentrancy-no-eth)
            vm.startPrank(deploymentArguments.deployerAddress, deploymentArguments.deployerAddress);
        } else if (isScript) {
            // forge-lint: disable-next-item(reentrancy-no-eth)
            vm.startBroadcast(deploymentArguments.privateKey);
        }

        console.log();

        //////////////////////////////////////////////////////////////////////////////

        console.log("ERC20 tokens:");

        if (!includeOrdinaryGearToken) {
            // Existing deployment profiles retain their verification-only token.
            erc20GearSupply = new ERC20GearSupply(deploymentArguments.deployerAddress, "MyToken", "MTK", 18);
        }

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
        if (includeOrdinaryGearToken) {
            // Reuse the existing CREATE slot; the queue and manager nonce geometry stays unchanged.
            address tokenMinter = vm.computeCreateAddress(
                deploymentArguments.deployerAddress, vm.getNonce(deploymentArguments.deployerAddress) + 9
            );
            erc20GearSupply = new ERC20GearSupply(tokenMinter, "Bridged Gear Origin Test", "GOT", 12);
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

        // forge-lint: disable-next-line(todo-comment)
        // TODO: `npm warn exec The following package was not found and will be installed: @openzeppelin/upgrades-core@x.y.z`
        if (!isFork) {
            wrappedVara = WrappedVara(
                Upgrades.deployUUPSProxy(
                    "WrappedVara.sol",
                    abi.encodeCall(
                        WrappedVara.initialize,
                        (
                            IGovernance(governanceAdminAddress),
                            IGovernance(governancePauserAddress),
                            ERC20Manager(erc20ManagerAddress)
                        )
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

        uint256 queueDeployerNonce = vm.getNonce(deploymentArguments.deployerAddress);
        address messageQueueAddress =
            vm.computeCreateAddress(deploymentArguments.deployerAddress, queueDeployerNonce + 4);
        if (expectedMessageQueueAddress != address(0)) {
            require(
                deploymentArguments.overrides.circleToken == BaseConstants.ZERO_ADDRESS
                    && deploymentArguments.overrides.tetherToken == BaseConstants.ZERO_ADDRESS
                    && deploymentArguments.overrides.wrappedEther == BaseConstants.ZERO_ADDRESS
                    && deploymentArguments.overrides.wrappedBitcoin == BaseConstants.ZERO_ADDRESS,
                "queue prediction requires no token overrides"
            );
            assertEq(messageQueueAddress, expectedMessageQueueAddress, "predicted queue address mismatch");
        }

        if (!isFork) {
            governanceAdmin = new GovernanceAdmin(
                deploymentArguments.governanceAdmin,
                wrappedVara,
                MessageQueue(messageQueueAddress),
                ERC20Manager(erc20ManagerAddress)
            );
        }
        console.log("    GovernanceAdmin:     ", address(governanceAdmin));

        if (!isFork) {
            assertEq(governanceAdminAddress, address(governanceAdmin));
        }

        if (!isFork) {
            governancePauser = new GovernancePauser(
                deploymentArguments.governancePauser,
                wrappedVara,
                MessageQueue(messageQueueAddress),
                ERC20Manager(erc20ManagerAddress)
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
            verifier = _deployVerifier(isTest, isScript, chainId, messageQueueAddress);
        }

        console.log("    Verifier:            ", address(verifier));

        // forge-lint: disable-next-line(todo-comment)
        // TODO: `npm warn exec The following package was not found and will be installed: @openzeppelin/upgrades-core@x.y.z`
        if (!isFork) {
            bytes memory queueInitialization;
            if (deploymentArguments.recoveryWallet == address(0)) {
                queueInitialization = abi.encodeCall(
                    MessageQueue.initialize,
                    (
                        governanceAdmin,
                        governancePauser,
                        deploymentArguments.emergencyStopAdmin,
                        deploymentArguments.emergencyStopObservers,
                        verifier
                    )
                );
            } else {
                queueInitialization = abi.encodeCall(
                    MessageQueue.initializeWithRecovery,
                    (
                        governanceAdmin,
                        governancePauser,
                        deploymentArguments.emergencyStopAdmin,
                        deploymentArguments.emergencyStopObservers,
                        verifier,
                        deploymentArguments.recoveryWallet
                    )
                );
            }
            messageQueue = MessageQueue(Upgrades.deployUUPSProxy("MessageQueue.sol", queueInitialization));
        }
        console.log("    MessageQueue:        ", address(messageQueue));

        messageQueueAssertions(isFork ? address(messageQueue) : messageQueueAddress);

        console.log();

        //////////////////////////////////////////////////////////////////////////////

        console.log("Bridge:");

        if (!isFork) {
            IERC20Manager.TokenInfo[] memory tokens = new IERC20Manager.TokenInfo[](includeOrdinaryGearToken ? 6 : 5);

            tokens[0] = IERC20Manager.TokenInfo(address(circleToken), IERC20Manager.TokenType.Ethereum);
            tokens[1] = IERC20Manager.TokenInfo(address(tetherToken), IERC20Manager.TokenType.Ethereum);
            tokens[2] = IERC20Manager.TokenInfo(address(wrappedEther), IERC20Manager.TokenType.Ethereum);
            tokens[3] = IERC20Manager.TokenInfo(address(wrappedVara), IERC20Manager.TokenType.Gear);
            tokens[4] = IERC20Manager.TokenInfo(address(wrappedBitcoin), IERC20Manager.TokenType.Ethereum);
            if (includeOrdinaryGearToken) {
                tokens[5] = IERC20Manager.TokenInfo(address(erc20GearSupply), IERC20Manager.TokenType.Gear);
            }

            erc20Manager = ERC20Manager(
                Upgrades.deployUUPSProxy(
                    "ERC20Manager.sol",
                    abi.encodeCall(
                        ERC20Manager.initialize,
                        (governanceAdmin, governancePauser, messageQueue, deploymentArguments.vftManager, tokens)
                    )
                )
            );
        }
        console.log("    ERC20Manager:        ", address(erc20Manager));

        erc20ManagerAssertions(isFork ? address(erc20Manager) : erc20ManagerAddress);

        //////////////////////////////////////////////////////////////////////////////

        console.log("Bridging payment:");

        if (!isFork) {
            // forge-lint: disable-next-item(reentrancy-no-eth)
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

    function _deployVerifier(bool isTest, bool isScript, uint256 chainId, address messageQueueAddress)
        internal
        virtual
        returns (IVerifier)
    {
        messageQueueAddress;
        if (isTest) {
            return new VerifierMock(true);
        }
        if (isScript) {
            if (chainId == 1) {
                return new VerifierMainnet();
            }
            return new VerifierTestnet();
        }
        return IVerifier(address(0));
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
        bool isMainnet = block.chainid == 1;
        uint256 expectedVftManagers = isFork() && isMainnet ? 3 : 1;
        assertEq(erc20Manager.totalVftManagers(), expectedVftManagers);
        bytes32[] memory vftManagers1 = erc20Manager.vftManagers();
        assertEq(vftManagers1.length, expectedVftManagers);
        assertEq(vftManagers1[expectedVftManagers - 1], deploymentArguments.vftManager);
        bytes32[] memory vftManagers2 = erc20Manager.vftManagers(expectedVftManagers, 1);
        assertEq(vftManagers2.length, 0);
        bytes32[] memory vftManagers3 = erc20Manager.vftManagers(expectedVftManagers - 1, 1);
        assertEq(vftManagers3.length, 1);
        assertEq(vftManagers3[0], deploymentArguments.vftManager);
        bytes32[] memory vftManagers4 = erc20Manager.vftManagers(0, 5);
        assertEq(vftManagers4.length, expectedVftManagers);
        assertEq(vftManagers4[expectedVftManagers - 1], deploymentArguments.vftManager);
        assertTrue(erc20Manager.isVftManager(deploymentArguments.vftManager));
        uint256 expectedTokens = includeOrdinaryGearToken ? 6 : 5;
        assertEq(erc20Manager.totalTokens(), expectedTokens);
        address[] memory tokens1 = erc20Manager.tokens();
        assertEq(tokens1.length, expectedTokens);
        assertEq(tokens1[0], address(circleToken));
        assertEq(tokens1[1], address(tetherToken));
        assertEq(tokens1[2], address(wrappedEther));
        assertEq(tokens1[3], address(wrappedVara));
        assertEq(tokens1[4], address(wrappedBitcoin));
        address[] memory tokens2 = erc20Manager.tokens(expectedTokens, 5);
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
        assertEq(tokens5.length, expectedTokens);
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
        if (includeOrdinaryGearToken) {
            assertEq(tokens1[5], address(erc20GearSupply));
            assertEq(tokens5[5], address(erc20GearSupply));
            assertEq(ERC20GearSupply(address(erc20GearSupply)).owner(), erc20ManagerAddress);
            assertTrue(erc20Manager.getTokenType(address(erc20GearSupply)) == IERC20Manager.TokenType.Gear);
        }
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
