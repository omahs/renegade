use ark_ec::PairingEngine;
use ark_ff::{PrimeField, Zero};
use ark_groth16::{ProvingKey, Proof, create_random_proof, generate_random_parameters};
use ark_r1cs_std::{prelude::{AllocVar, FieldVar, EqGadget, Boolean}, fields::fp::FpVar};
use ark_relations::r1cs::{ConstraintSynthesizer, SynthesisError, ConstraintSystemRef};
use arkworks_native_gadgets::{merkle_tree::{Path, SparseMerkleTree}, poseidon::Poseidon};
use arkworks_r1cs_gadgets::{merkle_tree::PathVar, poseidon::{PoseidonGadget, FieldHasherGadget}};
use num_bigint::BigUint;
use rand::rngs::OsRng;
use std::cell::RefCell;

use crate::{
    types::{
        Balance, Order, BalanceVar, OrderVar, SystemField, SystemPairingEngine, 
        SingleMatchResult, SingleMatchResultVar, WALLET_TREE_DEPTH, SystemHasher,
    }, 
    gadgets::{
        covered_match::ValidMatchGadget, 
        wallet_merkle::{get_merkle_hash_params, MerklePoseidonGadget}, poseidon::{BalanceHashInput, OrderHashInput}
    }
};


/**
 * Defines a smaller version of the ValidMatch circuit
 * that involves 4 low-depth Merkle proofs, and range checks
 */

pub struct SmallValidMatchCircuit {
    // Inputs
    matches: SingleMatchResult,
    balance1: Balance,
    balance2: Balance,
    order1: Order,
    order2: Order,

    // Circuit implementation
    circuit: RefCell<SmallValidMatchCircuitImpl<WALLET_TREE_DEPTH, SystemField>>
}

impl SmallValidMatchCircuit {
    pub fn new(
        balances_root: BigUint,
        orders_root: BigUint,
        matches: SingleMatchResult,
        balance1: Balance,
        balance2: Balance,
        balance1_opening: Path<SystemField, SystemHasher<SystemField>, WALLET_TREE_DEPTH>,
        balance2_opening: Path<SystemField, SystemHasher<SystemField>, WALLET_TREE_DEPTH>,
        order1: Order,
        order2: Order,
        order1_opening: Path<SystemField, SystemHasher<SystemField>, WALLET_TREE_DEPTH>,
        order2_opening: Path<SystemField, SystemHasher<SystemField>, WALLET_TREE_DEPTH>
    ) -> Self {
        let circuit = RefCell::new(
            SmallValidMatchCircuitImpl::new(
                balances_root.clone(),
                orders_root.clone(),
                matches.clone(), 
                balance1.clone(), 
                balance2.clone(), 
                balance1_opening.clone(),
                balance2_opening.clone(),
                order1.clone(), 
                order2.clone(),
                order1_opening.clone(),
                order2_opening.clone(),
            )
        );

        Self { matches, balance1, balance2, order1, order2, circuit }
    }

    // Creates a proving key for the proof system given a circuit size
    pub fn create_proving_key() -> Result<ProvingKey<SystemPairingEngine>, SynthesisError> {
        let mut rng = OsRng{};
        let dummy_circuit = SmallValidMatchCircuitImpl::<WALLET_TREE_DEPTH, _>::default();
        generate_random_parameters(dummy_circuit, &mut rng)
    }

    // Generates the circuit constraints using the witness supplied in the constructor
    pub fn generate_constraints(&self, cs: ConstraintSystemRef<SystemField>) -> Result<(), SynthesisError> {
        self.circuit
            .take()
            .generate_constraints(cs)
    }

    pub fn create_proof<E: PairingEngine<Fr = SystemField>>(
        &mut self, proving_key: &ProvingKey<E>
    ) -> Result<Proof<E>, SynthesisError> {
        // let circuit = self.wrapped_type.take();
        let mut rng = OsRng{};

        create_random_proof(self.circuit.take(), proving_key, &mut rng)
    }
}

#[derive(Clone)]
struct SmallValidMatchCircuitImpl<const TreeDepth: usize, F: PrimeField> {
    // Statement
    balances_root: BigUint,
    orders_root: BigUint,

    // Witness 
    matches: SingleMatchResult,
    balance1: Balance,
    balance2: Balance,
    balance1_opening: Path<F, SystemHasher<F>, TreeDepth>,
    balance2_opening: Path<F, SystemHasher<F>, TreeDepth>,
    order1: Order,
    order2: Order,
    order1_opening: Path<F, SystemHasher<F>, TreeDepth>,
    order2_opening: Path<F, SystemHasher<F>, TreeDepth>,
}

impl<const TreeDepth: usize, F: PrimeField> Default for SmallValidMatchCircuitImpl<TreeDepth, F> {
    fn default() -> Self {
        // Build a zero'd tree and fake hash 
        let empty_leaves = vec![F::zero(); 1 << (TreeDepth - 1)];
        let hasher = Poseidon::<F>::new(get_merkle_hash_params());
        let blank_tree = SparseMerkleTree::new_sequential(
            &empty_leaves, 
            &hasher, 
            &[0u8]
        ).unwrap();

        let default_path = blank_tree.generate_membership_proof(0);

        Self {
            // Statement variables
            balances_root: BigUint::zero(),
            orders_root: BigUint::zero(),
            // Witness variables
            matches: SingleMatchResult::default(),
            balance1: Balance::default(),
            balance2: Balance::default(),
            balance1_opening: default_path.clone(),
            balance2_opening: default_path.clone(),
            order1: Order::default(),
            order2: Order::default(),
            order1_opening: default_path.clone(),
            order2_opening: default_path,
        } 
    }
}

impl<const TreeDepth: usize, F: PrimeField> SmallValidMatchCircuitImpl<TreeDepth, F> {
   fn new(
        balances_root: BigUint,
        orders_root: BigUint,
        matches: SingleMatchResult,
        balance1: Balance,
        balance2: Balance,
        balance1_opening: Path<F, SystemHasher<F>, TreeDepth>,
        balance2_opening: Path<F, SystemHasher<F>, TreeDepth>,
        order1: Order,
        order2: Order,
        order1_opening: Path<F, SystemHasher<F>, TreeDepth>,
        order2_opening: Path<F, SystemHasher<F>, TreeDepth>
    ) -> Self {
        Self { 
            balances_root, 
            orders_root, 
            matches, 
            balance1, 
            balance2, 
            balance1_opening,
            balance2_opening,
            order1, 
            order2, 
            order1_opening,
            order2_opening,
        }
    }
}

impl<const TreeDepth: usize, F: PrimeField> ConstraintSynthesizer<F> for SmallValidMatchCircuitImpl<TreeDepth, F> {
    fn generate_constraints(
        self, 
        cs: ark_relations::r1cs::ConstraintSystemRef<F>
    ) -> Result<(), SynthesisError> {
        // Statement variables
        let balances_root_var = FpVar::new_input(
            cs.clone(), || { Ok( F::from(self.balances_root)) }
        )?;
        let orders_root_var = FpVar::new_input(
            cs.clone(), || { Ok( F::from(self.orders_root)) }
        )?;

        // Witness variables
        let single_match_var = SingleMatchResultVar::new_witness(cs.clone(), || { Ok(self.matches) })?;

        let balance1_var = BalanceVar::new_witness(cs.clone(), || { Ok(self.balance1) })?;
        let balance2_var = BalanceVar::new_witness(cs.clone(), || { Ok(self.balance2 )})?;

        let balance1_opening_var = PathVar::new_witness(cs.clone(), || { Ok(self.balance1_opening) })?;
        let balance2_opening_var = PathVar::new_witness(cs.clone(), || { Ok(self.balance2_opening )})?;

        let order1_var = OrderVar::new_witness(cs.clone(), || { Ok(self.order1) })?;
        let order2_var = OrderVar::new_witness(cs.clone(), || { Ok(self.order2) })?;

        let order1_opening_var = PathVar::new_witness(cs.clone(), || { Ok(self.order1_opening )})?;
        let order2_opening_var = PathVar::new_witness(cs.clone(), || { Ok(self.order2_opening )})?;

        // Check that
        // 1. The match is internally valid (sells match buys)
        // 2. The match is backed by two valid balances
        // 3. The match is a result of two valid orders
        ValidMatchGadget::enforce_valid(&single_match_var)?;
        ValidMatchGadget::enforce_valid_balances(&single_match_var, &balance1_var, &balance2_var)?;
        ValidMatchGadget::enforce_valid_orders(&single_match_var, &order1_var, &order2_var)?;

        // Check that the balances and orders are valid state elements
        let native_hasher = Poseidon::<F>::new(get_merkle_hash_params());
        MerklePoseidonGadget::check_opening(
            cs.clone(), 
            &Result::<BalanceHashInput<F>, SynthesisError>::from(&balance1_var)?, 
            native_hasher.clone(), 
            &balance1_opening_var, 
            &balances_root_var
        )?;

        MerklePoseidonGadget::check_opening(
            cs.clone(), 
            &Result::<BalanceHashInput<F>, SynthesisError>::from(&balance2_var)?, 
            native_hasher.clone(), 
            &balance2_opening_var, 
            &balances_root_var
        )?;
        
        MerklePoseidonGadget::check_opening(
            cs.clone(), 
            &Result::<OrderHashInput<F>, SynthesisError>::from(&order1_var)?, 
            native_hasher.clone(), 
            &order1_opening_var, 
            &orders_root_var
        )?;

        MerklePoseidonGadget::check_opening(
            cs.clone(), 
            &Result::<OrderHashInput<F>, SynthesisError>::from(&order2_var)?, 
            native_hasher, 
            &order2_opening_var, 
            &orders_root_var
        )?;

        // Validate the orders and balances in the state tree
        Ok(())
    }
}


#[cfg(test)]
mod small_valid_match_test {
    use ark_groth16::{prepare_verifying_key, verify_proof};
    use arkworks_native_gadgets::{poseidon::Poseidon, merkle_tree::SparseMerkleTree};

    use crate::{types::{Order, OrderSide, Balance, Match, SingleMatchResult, SystemField, WALLET_TREE_DEPTH}, gadgets::wallet_merkle::get_merkle_hash_params};

    use super::SmallValidMatchCircuit;

    #[test]
    fn test_match() {
        // Create fake overlapping orders with a midpoint price of 10 quote/base and 4 base tokens transferred
        let QUOTE_MINT = 1;
        let BASE_MINT = 2;

        let order1 = Order { quote_mint: QUOTE_MINT, base_mint: BASE_MINT, side: OrderSide::Buy, amount: 5, price: 11 };
        let order2 = Order { quote_mint: QUOTE_MINT, base_mint: BASE_MINT, side: OrderSide::Sell, amount: 3, price: 9 };

        let balance1 = Balance { mint: QUOTE_MINT, amount: 50 };
        let balance2 = Balance { mint: BASE_MINT, amount: 3 };

        let match_result = SingleMatchResult {
            buy_side1: Match { mint: BASE_MINT, amount: 3, side: OrderSide::Buy },
            sell_side1: Match { mint: QUOTE_MINT, amount: 30, side: OrderSide::Sell },
            buy_side2: Match { mint: QUOTE_MINT, amount: 30, side: OrderSide::Buy },
            sell_side2: Match { mint: BASE_MINT, amount: 3, side: OrderSide::Sell },
        };

        // Create fake Merkle openings for the balances and orders
        let hasher = Poseidon::<SystemField>::new(get_merkle_hash_params());
        let leaves = vec![
            SystemField::from(balance1.hash()),
            SystemField::from(balance2.hash()),
            SystemField::from(order1.hash()),
            SystemField::from(order2.hash())
        ];

        let tree = SparseMerkleTree::<SystemField, _, WALLET_TREE_DEPTH>::new_sequential(
            &leaves,
            &hasher,
            &[0u8; 32]
        ).unwrap();

        // Create a circuit and verify that it is satisfied
        let root = tree.root();
        let mut circuit = SmallValidMatchCircuit::new(
            root.into(),
            root.into(),
            match_result, 
            balance1, 
            balance2, 
            tree.generate_membership_proof(0 /* index */),
            tree.generate_membership_proof(1 /* index */),
            order1, 
            order2,
            tree.generate_membership_proof(2 /* index */),
            tree.generate_membership_proof(3 /* index */)
        );

        // Generate a proof
        let proving_key = SmallValidMatchCircuit::create_proving_key().unwrap();
        let proof = circuit.create_proof(&proving_key).unwrap();

        let verifying_key = prepare_verifying_key(&proving_key.vk);
        let verification_result = verify_proof(&verifying_key, &proof, &vec![
            root.clone(), root.clone()
        ]).unwrap();

        assert!(verification_result)
    }
}

