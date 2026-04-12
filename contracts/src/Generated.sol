// SPDX-License-Identifier: MIT
// Generated from FORML2 readings by AREST
pragma solidity ^0.8.20;

contract Order {
    struct Data {
        string id;
        bytes32 status;  // SM current state
    }

    mapping(string => Data) public records;

    event OrderWasPlacedByCustomer(string indexed order, string customer);

    // State Machine: 4 statuses
    // Statuses: InCart, Shipped, Archived, Placed
    modifier onlyInStatus(string memory id, bytes32 expected) {
        _onlyInStatus(id, expected);
        _;
    }

    function _onlyInStatus(string memory id, bytes32 expected) internal view {
        require(records[id].status == expected, "SM: wrong state");
    }

    function create(string memory id) external {
        require(bytes(records[id].id).length == 0, "UC: Order already exists");
        records[id].id = id;
        records[id].status = keccak256(bytes("In Cart"));
    }

    function archive(string memory id) external onlyInStatus(id, keccak256(bytes("Shipped"))) {
        records[id].status = keccak256(bytes("Archived"));
    }

    function ship(string memory id) external onlyInStatus(id, keccak256(bytes("Placed"))) {
        records[id].status = keccak256(bytes("Shipped"));
    }

    function place(string memory id) external onlyInStatus(id, keccak256(bytes("In Cart"))) {
        records[id].status = keccak256(bytes("Placed"));
    }
}

contract Customer {
    struct Data {
        string id;
        bytes32 status;  // SM current state
    }

    mapping(string => Data) public records;

    event OrderWasPlacedByCustomer(string indexed order, string customer);

    function create(string memory id) external {
        require(bytes(records[id].id).length == 0, "UC: Customer already exists");
        records[id].id = id;
    }
}
