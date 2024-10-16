pragma solidity ^0.8.24;

abstract contract BridgingPayment {
    event FeePaid();

    error NotAnAdmin();
    error NotEnoughFunds();

    address public underlying;

    uint256 fee;
    address admin;

    constructor(address _underlying, address _admin, uint256 _fee) payable {
        underlying = _underlying;
        admin = _admin;
        fee = _fee;
    }

    function deductFee() internal {
        (bool feeTransferSuccess, ) = admin.call{value: fee}("");
        if (!feeTransferSuccess) {
            revert NotEnoughFunds();
        }

        emit FeePaid();
    }

    function setFee(uint256 newFee) public {
        if (msg.sender != admin) {
            revert NotAnAdmin();
        } else {
            fee = newFee;
        }
    }

    function setAdmin(address newAdmin) public {
        if (msg.sender != admin) {
            revert NotAnAdmin();
        } else {
            admin = newAdmin;
        }
    }

    function getAdmin() public view returns (address) {
        return admin;
    }

    function getUnderlyingAddress() public view returns (address) {
        return address(underlying);
    }

    function getFee() public view returns (uint256) {
        return fee;
    }
}
