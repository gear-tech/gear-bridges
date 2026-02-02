// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.33;

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
import {UpgradeProxyMessage, GovernancePacker} from "src/interfaces/IGovernance.sol";
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

    using GovernancePacker for UpgradeProxyMessage;

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

            bool isMainnet = block.chainid == 1;
            if (isMainnet) {
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
            }

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

            // upgrade

            // new verifier
            IVerifier verifier_new = IVerifier(address(0xb7142E82cEeAd0df5D0b3507240A503E99E1881e));

            uint256 blockNumber_old = 28355670;
            bytes32 merkleRoot_old = 0x0000000000000000000000000000000000000000000000000000000000000000;
            bytes memory proof_old = bytes(
                hex"073a32d4cfea1067b4efc74d2dbe4c623c79dfdd5ff0bbdf3482fe79418a2b0b2c1effaa07c54d235c9e63048f088a9c3124b2e9bd4e7fa046b92292587153d50a5ae7da59373497c8af0a4fa84832d0f0c8203eca4cfe5e9fae0406844874320ecc7d58cf298c0687045fc8ec9f3f8273adbd7b0e6c74626f2e6be94b182d282679b4ca7520e6763df1518e83a954d3134f1e97852891d8f4ac600ce9695b550191de746967452c79bf3914ed58fcc6b875cbe8ea5d7d3b6e9a2f676b3f69f71f86e1e9c8792971a32135693fb89b2dd3d0779aa68cb2b5d18bac91da8d23140530b23f579f3cac10547f50c1ef1804ba208f4b03ca123fe44a26f11073514700fb43e61030b016a72a650c8ec3e77e785d616218f6901142046c33bc5cef742ad0c63f57303bf88a19545dd452fa2b410da2e16ee475e3ff259940ae8e40af29e001d74612df36f2727a372948e4bf23031a138a8ad1b876272e49b5f562c40c2d85d235fa10b6f823899730c8cf18eab83174de6b29614bfcde8e4ddc7e202691a90aae58224c56adfb93675a1e2ad96142dfcb2736308316f03a6ecea2370517b1b4be7bd1d05ca44cfd7528f6472c757b429d1a6123e70e829d5cd3e08614df30111397fbf0627421a66104193b481accc25f9b6a94ef592ecfd79577681ee9e0b09edb472082e81736373011ae368b19d31167b404ef4b34678f349f7221277a48c7d28662a18e8aa81014d4e3e57872156eb6b33bc43cf9fd29a699671d34eabb859d2f8eb457e8dba1f06809f51899db19c3c77cbaa1af535d9880ef210231b85de8f09eb30790d3c9e70090903242330ebf08ebd28d04e1e71623c42c6228886a920900cd3edebb334ed7fd9a10d0c969d3534ca95bc6a739645d03013e7bea3f239d06976c4cff5b06e8ada491f341bfd488cdd2607cf1cfd9afd80ac48e59f33778b9ebcba364d6977b9d72c437fae755bf1882729e3bdf58df3119e5242341b5ba9fc03f121861f6e9b50e446720d0889f47f82155643b9bebbf1a70555390cc8d94c01b919e2eb2a3b56d7485e3ef113584542db62a444abe8909b7f9d7647ae441a97afd4944b385c97649984a33abe7669973c461d81672031e90862f9a0a860916593c8ae2f1ba0697986ce8b11540eeb094c500b492f30a18e8c886ca78a3a9d3a7ba3dad5363f55cd4f52913b97ce28eb667964e19ed8018b6569eb7e94d907d73e356f79efbabebf2ccac11199316d5d9bc5a80e2de18271883dd90bf1fd45d5ee19f2ea23315992a75450c933db779697bcd4014f1f0"
            );

            uint256[] memory publicInputs_old = new uint256[](2);
            publicInputs_old[0] = uint256(merkleRoot_old) >> 64;
            publicInputs_old[1] = ((uint256(merkleRoot_old) & uint256(type(uint64).max)) << 128)
                | ((blockNumber_old & uint256(type(uint32).max)) << 96);

            assertFalse(verifier_new.safeVerifyProof(proof_old, publicInputs_old));

            // new proof
            uint256 blockNumber_new = 30068803;
            bytes32 merkleRoot_new = 0x0000000000000000000000000000000000000000000000000000000000000000;
            bytes memory proof_new = bytes(
                hex"11744d7e4aa34b139f632638e67c15e15d3d6b8c5f3b8b1f5da35186979f93342d2e0a31084631dc3fbab7581cc52123c99f5940cb3268b8d3afd307a7a457560aea7c2e0664b4c66e94299ac71a32c39e379efca9a1224df65086830a38399b1b6dc3d42d99778c93bb25196a7470949a56f22d91a0a75b01ff3c7b426a00e2285b507a391f06c9ff445fc43d102f8d508e03c0d8770ade9643615f250609fd21cde31ff99752542c8788aec88fd55de29c46194fa8e4541159d73631fa5eda2b66d93c4c8a4233cb2aa794a68aa4de2382317fac5737e09fd0566d7181cb5616b615d50783bc640769a93eaaf6f9ba7f37879b3fd9506b4ef1c990dd8c8639249a616d0f74de1fd776b442f7a5ec6ccd6887acbfdb2ab245af302dabb0361430609bb6dc15d6710c284febeb63a8c9e4a45ca1bd0d0c3d9240ebd8a6b97b2229f8f966ba98472bca0e4b5c686994f9f076d2707876d111ad8aa8c571cc60ef088a8c81cd6b2b20afef0b717f45f4c1540e4eacca6db3e1aec48f517c1ac6e413a3f163f7f75168cb5139d01eac74ff41fe3ee29198b7a4b1f86f596a8d7f0c2178910f107b27da7b4e3d241005463e209833827e5e23a1a5497e48511e9b8e1ab032a3b6630ea51ec48106f5caf4cfd7cb8589c6c37476dd6b1f173254af6111bb36f30adf07d8e044330730c9945d0db55984475b17cb7cc5af82c06b610d28aa80d85818b6a94ed5768020f93fecbf6c4d0228c440291c0ca0b4ef725acf1aeb46cfd76ded9f53d18b280b0faed10e76e9892976d14e55975b74dadf8c2b028ff024fa063852ed6026fc298a8b2c56760568ed7c73eaa9d7983df3bea3b907dc4960157423093b8c060427303d39c1bca0002f49ee5dfee002f66e494dba0269096b87b7a6d556065e42652ca84eb721a0558bf41e60ff4d96e0864d099324cdee76dc7bce7b4077a5a4409a172c8d3eeeca3ce7479b8e1a382931d4fc9d1626f4bfac278e4f9771c957028f9c5b8bc48a5ecbff34c975c3018a59d06db024b822223a2e8450f7f2d7343582b31fd79f8589bd3c5051e86b00ca51aacbc200e20a6c8d66e7fd73f49abb781c9a2aeb9bbcb704921b5dc5d0b73d014d3c6e157ab0509e198c92f66ac169549d404d778d0fdfdcfbb952ef3e9e5d9c357d322182a6b4d40bd37aea61c539dac7df355d7d77730c61d349de9bf319d67e8af20139f9366290ff174401b3a697e7137e23dcf551b4261306c9a59b886350a4ff260cc52236a29bed26f0fbe9a4a64f00f5287f4d6dbc1c96af3d80f211620c0a"
            );

            uint256[] memory publicInputs_new = new uint256[](2);
            publicInputs_new[0] = uint256(merkleRoot_new) >> 64;
            publicInputs_new[1] = ((uint256(merkleRoot_new) & uint256(type(uint64).max)) << 128)
                | ((blockNumber_new & uint256(type(uint32).max)) << 96);

            assertTrue(verifier_new.safeVerifyProof(proof_new, publicInputs_new));

            address newImplementation = address(0x256cbAEaA34ACFed715Fc60301E088575410CA63);

            VaraMessage memory message1 = VaraMessage({
                nonce: messageNonce++,
                source: governanceAdmin.governance(),
                destination: address(governanceAdmin),
                payload: UpgradeProxyMessage({
                    proxy: address(messageQueue),
                    newImplementation: newImplementation,
                    data: abi.encodeWithSelector(MessageQueue.reinitialize.selector)
                }).pack()
            });
            assertEq(messageQueue.isProcessed(message1.nonce), false);

            bytes32 messageHash = message1.hash();

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

            // test after upgrade

            vm.expectEmit(address(messageQueue));
            emit IMessageQueue.MerkleRoot(blockNumber_new, merkleRoot_new);

            messageQueue.submitMerkleRoot(blockNumber_new, merkleRoot_new, proof_new);

            // new verifier address
            assertEq(messageQueue.verifier(), address(0xb7142E82cEeAd0df5D0b3507240A503E99E1881e));

            // everything else should be kept the same
            assertEq(messageQueue.governanceAdmin(), address(0x3681A3e25F5652389B8f52504D517E96352830C3));
            assertEq(messageQueue.governancePauser(), address(0x257936C55518609E47eAab53f40a6e19437BEF47));

            verifier = IVerifier(messageQueue.verifier());
            vm.etch(address(verifier), type(VerifierMock).runtimeCode);
            VerifierMock(address(verifier)).setValue(true);
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
        // assertEq(messageQueue.verifier(), address(verifier));
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
