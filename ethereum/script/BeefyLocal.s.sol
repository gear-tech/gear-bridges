// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.37;

import {TimelockController} from "@openzeppelin/contracts/governance/TimelockController.sol";
import {Script} from "forge-std/Script.sol";
import {console} from "forge-std/console.sol";
import {MessageHandlerMock} from "src/mocks/MessageHandlerMock.sol";
import {DeploymentBase, DeploymentArguments} from "script/DeploymentBase.sol";
import {BaseConstants} from "test/BaseConstants.sol";

interface ILocalRecoverySafe {
    function setup(
        address[] calldata owners,
        uint256 threshold,
        address to,
        bytes calldata data,
        address fallbackHandler,
        address paymentToken,
        uint256 payment,
        address payable paymentReceiver
    ) external;
}

contract BeefyLocal is Script, DeploymentBase {
    // Test-only identities; the bridge deployer's CREATE sequence is unchanged.
    address public recoverySafeSingleton;
    address public recoverySafe;
    address public recoveryTimelock;

    function _localDeploymentFixture() internal view override returns (bool) {
        return block.chainid == 31337;
    }
    function run()
        public
        returns (address clientAddress, address verifierAddress, address queueAddress, address receiverAddress)
    {
        require(block.chainid == 31337, "BeefyLocal requires chain id 31337");
        uint256 privateKey = vm.envUint("PRIVATE_KEY");
        address deployerAddress = vm.addr(privateKey);
        address[] memory observers = new address[](2);
        observers[0] = BaseConstants.EMERGENCY_STOP_OBSERVER1;
        observers[1] = BaseConstants.EMERGENCY_STOP_OBSERVER2;
        DeploymentArguments memory args = DeploymentArguments({
            privateKey: privateKey,
            deployerAddress: deployerAddress,
            forkUrlOrAlias: "",
            overrides: _tokenOverridesFromEnvironment(),
            vftManager: vm.envOr("LOCAL_NATIVE_MANAGER", BaseConstants.VFT_MANAGER),
            governanceAdmin: BaseConstants.GOVERNANCE_ADMIN,
            governancePauser: BaseConstants.GOVERNANCE_PAUSER,
            emergencyStopAdmin: BaseConstants.EMERGENCY_STOP_ADMIN,
            emergencyStopObservers: observers,
            bridgingPaymentFee: BaseConstants.BRIDGING_PAYMENT_FEE
        });
        deployTestTokenProfile = true; // Explicit local test profile, never a production token fallback.
        _validateDeploymentArguments(args);
        deploymentArguments = args;
        address predictedQueue = _predictBeefyMessageQueue(deployerAddress, args.overrides);
        BeefyBootstrap memory bootstrap = _loadBeefyBootstrap(predictedQueue);
        uint256 recoveryKey = vm.envOr("LOCAL_RECOVERY_PRIVATE_KEY", uint256(0));
        if (recoveryKey != 0) {
            _deployLocalRecovery(recoveryKey, deployerAddress);
        }
        _deployBeefyClient(privateKey, predictedQueue, bootstrap);
        deployBridge(args);
        require(address(messageQueue) == predictedQueue, "deployed queue differs from prediction");

        vm.startBroadcast(privateKey);
        MessageHandlerMock receiver = new MessageHandlerMock();
        vm.stopBroadcast();
        return (address(beefyClient), address(verifier), address(messageQueue), address(receiver));
    }

    function _deployLocalRecovery(uint256 recoveryKey, address deployerAddress) private {
        address[] memory owners = new address[](2);
        owners[0] = vm.addr(recoveryKey);
        owners[1] = vm.addr(vm.envUint("LOCAL_RECOVERY_OWNER_PRIVATE_KEY"));
        require(owners[0] != deployerAddress, "fixture recovery must use a separate signer");
        require(owners[1] != owners[0] && owners[1] != deployerAddress, "fixture requires two separate recovery owners");

        // Unmodified published artifacts, not getter mocks or locally rebuilt substitutes.
        string memory singletonArtifact = vm.readFile("test/fixtures/safe-1.4.1/Safe.json");
        string memory proxyArtifact = vm.readFile("test/fixtures/safe-1.4.1/SafeProxy.json");
        require(
            sha256(bytes(singletonArtifact)) == 0xc36a5e99bc3f25c2d75ac5f7920b679200b3ce617c0d26e674b1321dbc6b0bdf
                && sha256(bytes(proxyArtifact)) == 0xb05eaeaf7278097e52a9e9b38410de2a812c23fa3622373473e73eaa19646ecd,
            "pinned Safe fixture artifact changed"
        );
        bytes memory singletonCreation = vm.parseJsonBytes(singletonArtifact, ".bytecode");
        bytes memory proxyCreation = vm.parseJsonBytes(proxyArtifact, ".bytecode");
        vm.startBroadcast(recoveryKey);
        recoverySafeSingleton = _createFixture(singletonCreation);
        recoverySafe = _createFixture(bytes.concat(proxyCreation, abi.encode(recoverySafeSingleton)));
        ILocalRecoverySafe(recoverySafe).setup(owners, 2, address(0), "", address(0), address(0), 0, payable(address(0)));
        address[] memory proposers = new address[](1);
        proposers[0] = recoverySafe;
        // OZ grants proposer+canceller to Safe, executor to Safe, and admin only to itself.
        recoveryTimelock = address(new TimelockController(1 hours, proposers, proposers, address(0)));
        vm.stopBroadcast();
        require(
            recoverySafeSingleton.codehash == keccak256(vm.parseJsonBytes(singletonArtifact, ".deployedBytecode"))
                && recoverySafe.codehash == keccak256(vm.parseJsonBytes(proxyArtifact, ".deployedBytecode")),
            "deployed Safe fixture bytecode mismatch"
        );
        vm.setEnv("BEEFY_RECOVERY_TIMELOCK", vm.toString(recoveryTimelock));
        vm.setEnv("LOCAL_RECOVERY_SAFE", vm.toString(recoverySafe));
        vm.setEnv("LOCAL_RECOVERY_SAFE_SINGLETON", vm.toString(recoverySafeSingleton));
        console.log("LOCAL_RECOVERY_SAFE_SINGLETON", recoverySafeSingleton);
        console.log("LOCAL_RECOVERY_SAFE", recoverySafe);
        console.log("BEEFY_RECOVERY_TIMELOCK", recoveryTimelock);
    }

    function _createFixture(bytes memory creationCode) private returns (address deployed) {
        assembly ("memory-safe") {
            deployed := create(0, add(creationCode, 0x20), mload(creationCode))
        }
        require(deployed != address(0), "Safe fixture CREATE failed");
    }
}
