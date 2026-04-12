// SPDX-License-Identifier: MIT
// Generated from FORML2 readings by AREST
pragma solidity ^0.8.20;

import {Script, console} from "forge-std/Script.sol";
import {Customer, Order} from "../src/Generated.sol";

/// Deploy every user contract declared in readings/.
/// Invoke: forge script script/Deploy.s.sol --rpc-url $RPC --private-key $KEY --broadcast
contract Deploy is Script {
    function run() external {
        vm.startBroadcast();
        Customer customer = new Customer();
        console.log("Customer deployed at:", address(customer));
        Order order = new Order();
        console.log("Order deployed at:", address(order));
        vm.stopBroadcast();
    }
}

