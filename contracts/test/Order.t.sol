// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {Order} from "../src/Generated.sol";

contract OrderTest is Test {
    Order internal orderContract;

    function setUp() public {
        orderContract = new Order();
    }

    // An Order created starts in the "In Cart" status.
    function test_CreateAssignsInitialStatus() public {
        orderContract.create("ord-1");
        (string memory id, bytes32 status) = orderContract.records("ord-1");
        assertEq(id, "ord-1");
        assertEq(status, keccak256(bytes("In Cart")));
    }

    // Creating the same Order twice reverts on UC.
    function test_CreateRevertsOnDuplicate() public {
        orderContract.create("ord-1");
        vm.expectRevert(bytes("UC: Order already exists"));
        orderContract.create("ord-1");
    }

    // place() advances In Cart → Placed.
    function test_PlaceAdvancesStatus() public {
        orderContract.create("ord-1");
        orderContract.place("ord-1");
        (, bytes32 status) = orderContract.records("ord-1");
        assertEq(status, keccak256(bytes("Placed")));
    }

    // ship() from In Cart is rejected: wrong state.
    function test_ShipFromInCartReverts() public {
        orderContract.create("ord-1");
        vm.expectRevert(bytes("SM: wrong state"));
        orderContract.ship("ord-1");
    }

    // Full lifecycle: Create → place → ship → archive.
    function test_FullLifecycle() public {
        orderContract.create("ord-2");
        orderContract.place("ord-2");
        orderContract.ship("ord-2");
        orderContract.archive("ord-2");
        (, bytes32 status) = orderContract.records("ord-2");
        assertEq(status, keccak256(bytes("Archived")));
    }

    // Attempting to archive before ship is rejected.
    function test_ArchiveBeforeShipReverts() public {
        orderContract.create("ord-3");
        orderContract.place("ord-3");
        vm.expectRevert(bytes("SM: wrong state"));
        orderContract.archive("ord-3");
    }
}
