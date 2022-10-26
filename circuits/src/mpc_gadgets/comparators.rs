//! Groups logic around arithemtic comparator circuits

use std::ops::Neg;

use curve25519_dalek::scalar::Scalar;
use mpc_ristretto::{
    authenticated_scalar::AuthenticatedScalar, beaver::SharedValueSource, network::MpcNetwork,
};

use crate::{errors::MpcError, mpc::SharedFabric};

use super::{
    arithmetic::product,
    bits::{bit_xor, scalar_from_bits_le, scalar_to_bits_le, to_bits_le},
    modulo::truncate,
};

/// Implements the comparator a < 0
///
/// D represents is the bitlength of the input
pub fn less_than_zero<const D: usize, N: MpcNetwork + Send, S: SharedValueSource<Scalar>>(
    a: &AuthenticatedScalar<N, S>,
    fabric: SharedFabric<N, S>,
) -> Result<AuthenticatedScalar<N, S>, MpcError> {
    Ok(truncate(a, D - 1, fabric)?.neg())
}

/// Implements the comparator a == 0
///
/// D represents the bitlength of the input
#[allow(unused_variables)]
pub fn eq_zero<const D: usize, N: MpcNetwork + Send, S: SharedValueSource<Scalar>>(
    a: &AuthenticatedScalar<N, S>,
    fabric: SharedFabric<N, S>,
) -> Result<AuthenticatedScalar<N, S>, MpcError> {
    let bits = to_bits_le::<D, N, S>(a, fabric.clone())?;
    Ok(Scalar::one() - kary_or(&bits, fabric)?)
}

/// Implements the comparator a == b
///
/// D represents the bitlength of the inputs
pub fn eq<const D: usize, N: MpcNetwork + Send, S: SharedValueSource<Scalar>>(
    a: &AuthenticatedScalar<N, S>,
    b: &AuthenticatedScalar<N, S>,
    fabric: SharedFabric<N, S>,
) -> Result<AuthenticatedScalar<N, S>, MpcError> {
    eq_zero::<D, N, S>(&(a - b), fabric)
}

/// Implements the comparator a != b
///
/// D represents the bitlength of the inputs
pub fn ne<const D: usize, N: MpcNetwork + Send, S: SharedValueSource<Scalar>>(
    a: &AuthenticatedScalar<N, S>,
    b: &AuthenticatedScalar<N, S>,
    fabric: SharedFabric<N, S>,
) -> Result<AuthenticatedScalar<N, S>, MpcError> {
    Ok(Scalar::one() - eq::<D, N, S>(a, b, fabric)?)
}

/// Implements the comparator a < b
///
/// D represents the bitlength of a and b
pub fn less_than<const D: usize, N: MpcNetwork + Send, S: SharedValueSource<Scalar>>(
    a: &AuthenticatedScalar<N, S>,
    b: &AuthenticatedScalar<N, S>,
    fabric: SharedFabric<N, S>,
) -> Result<AuthenticatedScalar<N, S>, MpcError> {
    less_than_zero::<D, _, _>(&(a - b), fabric)
}

/// Implements the comparator a <= b
///
/// D represents the bitlength of a and b
pub fn less_than_equal<const D: usize, N: MpcNetwork + Send, S: SharedValueSource<Scalar>>(
    a: &AuthenticatedScalar<N, S>,
    b: &AuthenticatedScalar<N, S>,
    fabric: SharedFabric<N, S>,
) -> Result<AuthenticatedScalar<N, S>, MpcError> {
    Ok(Scalar::one() - greater_than::<D, _, _>(a, b, fabric)?)
}

/// Implements the comparator a > b
///
/// D represents the bitlength of a and b
pub fn greater_than<const D: usize, N: MpcNetwork + Send, S: SharedValueSource<Scalar>>(
    a: &AuthenticatedScalar<N, S>,
    b: &AuthenticatedScalar<N, S>,
    fabric: SharedFabric<N, S>,
) -> Result<AuthenticatedScalar<N, S>, MpcError> {
    less_than_zero::<D, _, _>(&(b - a), fabric)
}

/// Implements the comparator a >= b
///
/// D represents the bitlength of a and b
pub fn greater_than_equal<const D: usize, N: MpcNetwork + Send, S: SharedValueSource<Scalar>>(
    a: &AuthenticatedScalar<N, S>,
    b: &AuthenticatedScalar<N, S>,
    fabric: SharedFabric<N, S>,
) -> Result<AuthenticatedScalar<N, S>, MpcError> {
    Ok(Scalar::one() - less_than::<D, _, _>(a, b, fabric)?)
}

/// Implements a k-ary Or comparator; i.e. a_1 || a_2 || ... || a_n
///
/// This method works as follows to achieve a constant round scheme:
///     1. Sum the a_i values
///     2. Blind this sum, open the blinded value, and decompose to bits
///     3. xor the bits with their blinding bits to recover shared bits
///     4. Compute an "or" over the recovered bits
pub fn kary_or<N: MpcNetwork + Send, S: SharedValueSource<Scalar>>(
    a: &[AuthenticatedScalar<N, S>],
    fabric: SharedFabric<N, S>,
) -> Result<AuthenticatedScalar<N, S>, MpcError> {
    // Sample random blinding bits from the pre-processing functionality
    // We only need to be able to hold the maximum possible count, log_2(# of booleans)
    let max_bits = ((a.len() + 1) as f32).log2().ceil() as usize;
    let blinding_bits = fabric
        .borrow_fabric()
        .allocate_random_shared_bit_batch(max_bits);
    let blinding_value = scalar_from_bits_le(&blinding_bits);
    let blinding_value_upper_bits = Scalar::from((1 << max_bits) as u64)
        * fabric.borrow_fabric().allocate_random_shared_scalar();

    // Blind the sum of all booleans and open the result
    let sum_a = a.iter().sum();
    let blinded_sum = &sum_a + &blinding_value + &blinding_value_upper_bits;

    let blinded_sum_open = blinded_sum
        .open_and_authenticate()
        .map_err(|err| MpcError::OpeningError(err.to_string()))?;

    // Decompose the blinded sum into bits
    let blinded_sum_bits = scalar_to_bits_le(blinded_sum_open.to_scalar())
        .into_iter()
        .map(|val| fabric.borrow_fabric().allocate_public_scalar(val));

    // XOR the blinded sum bits with the blinding bits that are still shared to obtain a sharing of the
    // sum bits (unblinded)
    let unblinded_shared_bits = blinded_sum_bits
        .zip(blinding_bits.iter())
        .map(|(blinded_bit, blinder_bit)| bit_xor(&blinded_bit, blinder_bit))
        .collect::<Vec<_>>();

    constant_round_or_impl(&unblinded_shared_bits, fabric)
}

// TODO: Optimize this into parallel blocks for larger lenth inputs
/// Computes the "OR" of all input bits using a public polynomial.
///
/// Specifically, the method evaluates the polynomial:
///     f(x) = 1/n! * (1 - x) * (2 - x) * ... * (n - x)
/// which is zero at 1..n and 1 at 0. Then we take 1 - f(x) to flip the result
///
/// This effectively maps any non-zero count to 1 and zero to 0
fn constant_round_or_impl<N: MpcNetwork + Send, S: SharedValueSource<Scalar>>(
    a: &[AuthenticatedScalar<N, S>],
    fabric: SharedFabric<N, S>,
) -> Result<AuthenticatedScalar<N, S>, MpcError> {
    // Sum up the booleans
    let n = a.len();
    let sum_a: AuthenticatedScalar<N, S> = a.iter().sum();

    // Compute (1 - sum) * (2 - sum) * ... * (n - sum) / n!
    // We wrap the n! in implicitly here to avoid overflow computing n! directly for large n
    let sum_monomials = (1..n + 1)
        .map(|x| (Scalar::from(x as u64) - &sum_a) * Scalar::from(x as u64).invert())
        .collect::<Vec<_>>();

    let monomial_product = product(&sum_monomials, fabric)
        .map_err(|err| MpcError::ArithmeticError(err.to_string()))?;

    // Flip the result
    Ok(Scalar::one() - monomial_product)
}

/// TODO: Optimize this method
/// Computes the min of two scalars
///
/// D represents the bitlength of a and b
pub fn min<const D: usize, N: MpcNetwork + Send, S: SharedValueSource<Scalar>>(
    a: &AuthenticatedScalar<N, S>,
    b: &AuthenticatedScalar<N, S>,
    fabric: SharedFabric<N, S>,
) -> Result<AuthenticatedScalar<N, S>, MpcError> {
    let a_lt_b = less_than::<D, _, _>(a, b, fabric)?;
    Ok(&a_lt_b * a + (Scalar::one() - a_lt_b) * b)
}

/// Computes res = a if s else b
pub fn cond_select<N: MpcNetwork + Send, S: SharedValueSource<Scalar>>(
    s: &AuthenticatedScalar<N, S>,
    a: &AuthenticatedScalar<N, S>,
    b: &AuthenticatedScalar<N, S>,
) -> Result<AuthenticatedScalar<N, S>, MpcError> {
    let selectors =
        AuthenticatedScalar::batch_mul(&[a.clone(), b.clone()], &[s.clone(), Scalar::one() - s])
            .map_err(|err| MpcError::ArithmeticError(err.to_string()))?;
    Ok(&selectors[0] + &selectors[1])
}
