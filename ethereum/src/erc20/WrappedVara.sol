// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
pragma solidity ^0.8.30;

import {AccessControlUpgradeable} from "@openzeppelin/contracts-upgradeable/access/AccessControlUpgradeable.sol";
import {ERC20Upgradeable} from "@openzeppelin/contracts-upgradeable/token/ERC20/ERC20Upgradeable.sol";
import {
    ERC20BurnableUpgradeable
} from "@openzeppelin/contracts-upgradeable/token/ERC20/extensions/ERC20BurnableUpgradeable.sol";
import {
    ERC20PausableUpgradeable
} from "@openzeppelin/contracts-upgradeable/token/ERC20/extensions/ERC20PausableUpgradeable.sol";
import {
    ERC20PermitUpgradeable
} from "@openzeppelin/contracts-upgradeable/token/ERC20/extensions/ERC20PermitUpgradeable.sol";
import {Initializable} from "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import {UUPSUpgradeable} from "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import {IERC20Mintable} from "src/interfaces/IERC20Mintable.sol";
import {IGovernance} from "src/interfaces/IGovernance.sol";
import {IPausable} from "src/interfaces/IPausable.sol";

/**
 * @dev Wrapped Vara (WVARA) is represents VARA on Ethereum as ERC20 token.
 *      VARA is also used for paying fees, staking and governance on Vara Network,
 *      while WVARA does all of the same things but on Ethereum.
 */
contract WrappedVara is
    Initializable,
    ERC20Upgradeable,
    ERC20BurnableUpgradeable,
    ERC20PausableUpgradeable,
    AccessControlUpgradeable,
    ERC20PermitUpgradeable,
    UUPSUpgradeable,
    IERC20Mintable,
    IPausable
{
    bytes32 public constant PAUSER_ROLE = bytes32(uint256(0x01));
    bytes32 public constant MINTER_ROLE = bytes32(uint256(0x02));

    string private constant TOKEN_NAME_MAINNET = "Bridged Wrapped Vara";
    string private constant TOKEN_NAME_TESTNET = "Bridged Wrapped Testnet Vara";

    string private constant TOKEN_SYMBOL_MAINNET = "WVARA";
    string private constant TOKEN_SYMBOL_TESTNET = "WTVARA";

    IGovernance private _governanceAdmin;
    IGovernance private _governancePauser;
    address private _minter;

    /**
     * @custom:oz-upgrades-unsafe-allow constructor
     */
    constructor() {
        _disableInitializers();
    }

    /**
     * @dev Initializes the WrappedVara contract with the token name and symbol.
     * @param governanceAdmin_ The address of the GovernanceAdmin contract that will process messages.
     * @param governancePauser_ The address of the GovernanceAdmin contract that will process pauser messages.
     * @param minter_ The address that will be able to mint tokens.
     */
    function initialize(IGovernance governanceAdmin_, IGovernance governancePauser_, address minter_)
        public
        initializer
    {
        bool isMainnet = block.chainid == 1;

        string memory tokenName = isMainnet ? TOKEN_NAME_MAINNET : TOKEN_NAME_TESTNET;
        string memory tokenSymbol = isMainnet ? TOKEN_SYMBOL_MAINNET : TOKEN_SYMBOL_TESTNET;

        __ERC20_init(tokenName, tokenSymbol);
        __ERC20Burnable_init();
        __ERC20Pausable_init();
        __AccessControl_init();
        __ERC20Permit_init(tokenName);
        __UUPSUpgradeable_init();

        _grantRole(DEFAULT_ADMIN_ROLE, address(governanceAdmin_));

        _grantRole(PAUSER_ROLE, address(governanceAdmin_));
        _grantRole(PAUSER_ROLE, address(governancePauser_));

        _grantRole(MINTER_ROLE, minter_);

        _governanceAdmin = governanceAdmin_;
        _governancePauser = governancePauser_;
        _minter = minter_;
    }

    /**
     * @custom:oz-upgrades-validate-as-initializer
     */
    // function reinitialize() public onlyRole(DEFAULT_ADMIN_ROLE) reinitializer(2) {}

    /**
     * @dev Returns governance admin address.
     * @return governanceAdmin Governance admin address.
     */
    function governanceAdmin() external view returns (address) {
        return address(_governanceAdmin);
    }

    /**
     * @dev Returns governance pauser address.
     * @return governancePauser Governance pauser address.
     */
    function governancePauser() external view returns (address) {
        return address(_governancePauser);
    }

    /**
     * @dev Returns minter address.
     * @return minter Minter address.
     */
    function minter() external view returns (address) {
        return _minter;
    }

    /**
     * @dev Returns the number of decimals used to get its user representation.
     *      Also see documentation about decimals:
     *      - https://wiki.vara.network/docs/staking/validator-faqs#what-is-the-precision-of-the-vara-token
     */
    function decimals() public view virtual override returns (uint8) {
        return 12;
    }

    /**
     * @dev Pauses the contract.
     */
    function pause() public onlyRole(PAUSER_ROLE) {
        _pause();
    }

    /**
     * @dev Unpauses the contract.
     */
    function unpause() public onlyRole(PAUSER_ROLE) {
        _unpause();
    }

    /**
     * @dev Mints `amount` tokens to `to`.
     * @param to The address to mint tokens to.
     * @param amount The amount of tokens to mint.
     */
    function mint(address to, uint256 amount) public onlyRole(MINTER_ROLE) {
        _mint(to, amount);
    }

    /**
     * @dev Function that should revert when `msg.sender` is not authorized to upgrade the contract.
     *      Called by {upgradeToAndCall}.
     */
    function _authorizeUpgrade(address newImplementation) internal override onlyRole(DEFAULT_ADMIN_ROLE) {}

    // The following functions are overrides required by Solidity.

    function _update(address from, address to, uint256 value)
        internal
        override(ERC20Upgradeable, ERC20PausableUpgradeable)
    {
        super._update(from, to, value);
    }
}
