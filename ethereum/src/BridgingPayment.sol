pragma solidity ^0.8.24;

abstract contract BridgingPayment {
    event FeePaid();

    error NotAnAdmin();

    address public underlying;

    uint256 fee;
    address payable admin;

    constructor(address _underlying, address _admin, uint256 _fee) payable {
        underlying = _underlying;
        admin = payable(_admin);
        fee = _fee;
    }

    /** @dev Deduct a `fee` from the user and transfer it to the `admin` address.
     *  This function reverts if user don't have enough funds to pay the fee.
     */
    function deductFee() internal {
        admin.transfer(fee);

        emit FeePaid();
    }

    /** @dev Set fee that'll be deducted from user when he sends requests to the contract.
     * This function can be called only by an admin.
     *
     * @param newFee new fee amount
     */
    function setFee(uint256 newFee) public {
        if (msg.sender != admin) {
            revert NotAnAdmin();
        } else {
            fee = newFee;
        }
    }

    /** @dev Set new admin for a contract. This function can be called only by an admin.
     *
     * @param newAdmin new admin address
     */
    function setAdmin(address newAdmin) public {
        if (msg.sender != admin) {
            revert NotAnAdmin();
        } else {
            admin = payable(newAdmin);
        }
    }

    /** @dev Get current admin address. */
    function getAdmin() public view returns (address) {
        return admin;
    }

    /** @dev Get address of the contract that will be called when sending request to `BridgingPayment`. */
    function getUnderlyingAddress() public view returns (address) {
        return underlying;
    }

    /** @dev Get current fee amount. */
    function getFee() public view returns (uint256) {
        return fee;
    }
}
