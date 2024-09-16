use super::utils::{Error, Result, *};
use sails_rs::prelude::*;

pub fn allowance(allowances: &AllowancesMap, owner: ActorId, spender: ActorId) -> U256 {
    allowances
        .get(&(owner, spender))
        .cloned()
        .unwrap_or_default()
}

pub fn approve(
    allowances: &mut AllowancesMap,
    owner: ActorId,
    spender: ActorId,
    value: U256,
) -> bool {
    if owner == spender {
        return false;
    }

    let key = (owner, spender);

    if value.is_zero() {
        return allowances.remove(&key).is_some();
    }

    let prev = allowances.insert(key, value);

    prev.map(|v| v != value).unwrap_or(true)
}

pub fn balance_of(balances: &BalancesMap, owner: ActorId) -> U256 {
    balances.get(&owner).cloned().unwrap_or_default()
}

pub fn transfer(
    balances: &mut BalancesMap,
    from: ActorId,
    to: ActorId,
    value: U256,
) -> Result<bool> {
    if from == to || value.is_zero() {
        return Ok(false);
    }

    let new_from = balance_of(balances, from)
        .checked_sub(value)
        .ok_or(Error::InsufficientBalance)?;

    let new_to = balance_of(balances, to)
        .checked_add(value)
        .ok_or(Error::NumericOverflow)?;

    if !new_from.is_zero() {
        balances.insert(from, new_from);
    } else {
        balances.remove(&from);
    }

    balances.insert(to, new_to);

    Ok(true)
}

pub fn transfer_from(
    allowances: &mut AllowancesMap,
    balances: &mut BalancesMap,
    spender: ActorId,
    from: ActorId,
    to: ActorId,
    value: U256,
) -> Result<bool> {
    if spender == from {
        return transfer(balances, from, to, value);
    }

    if from == to || value.is_zero() {
        return Ok(false);
    };

    let new_allowance = allowance(allowances, from, spender)
        .checked_sub(value)
        .ok_or(Error::InsufficientAllowance)?;

    let _res = transfer(balances, from, to, value)?;
    debug_assert!(_res);

    let key = (from, spender);

    if !new_allowance.is_zero() {
        allowances.insert(key, new_allowance);
    } else {
        allowances.remove(&key);
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::funcs;
    use utils::*;

    macro_rules! assert_ok {
        ( $x:expr, $y: expr $(,)? ) => {{
            assert_eq!($x.unwrap(), $y);
        }};
    }

    macro_rules! assert_err {
        ( $x:expr, $y: expr $(,)? ) => {{
            assert_eq!($x.err().expect("Ran into Ok value"), $y);
        }};
    }

    #[test]
    fn allowance() {
        // Initializing thread logger.
        let _ = env_logger::try_init();

        // Creating map with one single approve from Alice to Bob.
        let map = allowances_map([(alice(), bob(), U256::exp10(42))]);

        // # Test case #1.
        // Approve is returned if exists.
        {
            assert!(map.contains_key(&(alice(), bob())));
            assert_eq!(funcs::allowance(&map, alice(), bob()), U256::exp10(42));
        }

        // # Test case #2.
        // U256::zero() is returned if not exists.
        {
            assert!(!map.contains_key(&(bob(), alice())));
            assert!(funcs::allowance(&map, bob(), alice()).is_zero());
        }
    }

    #[test]
    fn approve() {
        // Initializing thread logger.
        let _ = env_logger::try_init();

        // Creating empty map.
        let mut map = allowances_map([]);
        assert!(map.is_empty());

        // # Test case #1.
        // Allowance from Alice to Bob doesn't exist and created.
        {
            assert!(funcs::approve(&mut map, alice(), bob(), U256::exp10(42)));
            assert_eq!(funcs::allowance(&map, alice(), bob()), U256::exp10(42));
        }

        // # Test case #2.
        // Allowance from Alice to Bob exist and changed.
        {
            assert!(funcs::approve(&mut map, alice(), bob(), U256::exp10(24)));
            assert_eq!(funcs::allowance(&map, alice(), bob()), U256::exp10(24));
        }

        // # Test case #3.
        // Allowance from Alice to Bob exists and not changed.
        {
            assert!(!funcs::approve(&mut map, alice(), bob(), U256::exp10(24)));
            assert_eq!(funcs::allowance(&map, alice(), bob()), U256::exp10(24));
        }

        // # Test case #4.
        // Allowance from Alice to Bob exists and removed.
        {
            assert!(funcs::approve(&mut map, alice(), bob(), U256::zero()));
            assert!(funcs::allowance(&map, alice(), bob()).is_zero());
        }

        // # Test case #5.
        // Allowance from Alice to Bob doesn't exists and not created.
        {
            assert!(!funcs::approve(&mut map, alice(), bob(), U256::zero()));
            assert!(funcs::allowance(&map, alice(), bob()).is_zero());
        }

        // # Test case #6.
        // Allowance is always noop on owner == spender.
        {
            assert!(!funcs::approve(&mut map, alice(), alice(), U256::exp10(42)));
            assert!(funcs::allowance(&map, alice(), alice()).is_zero());

            assert!(!funcs::approve(&mut map, alice(), alice(), U256::exp10(24)));
            assert!(funcs::allowance(&map, alice(), alice()).is_zero());

            assert!(!funcs::approve(&mut map, alice(), alice(), U256::zero()));
            assert!(funcs::allowance(&map, alice(), alice()).is_zero());
        }
    }

    #[test]
    fn balance_of() {
        // Initializing thread logger.
        let _ = env_logger::try_init();

        // Creating map with one single balance belonged to Alice.
        let map = balances_map([(alice(), U256::exp10(42))]);

        // # Test case #1.
        // Balance is returned if exists.
        {
            assert!(map.contains_key(&alice()));
            assert_eq!(funcs::balance_of(&map, alice()), U256::exp10(42));
        }

        // # Test case #2.
        // U256::zero() is returned if not exists.
        {
            assert!(!map.contains_key(&bob()));
            assert!(funcs::balance_of(&map, bob()).is_zero());
        }
    }

    #[test]
    fn transfer() {
        // Initializing thread logger.
        let _ = env_logger::try_init();

        // Creating map with medium balance belonged to Bob and max one to Dave.
        let mut map = balances_map([(bob(), U256::exp10(42)), (dave(), U256::MAX)]);

        // # Test case #1.
        // Alice transfers to Bob, when Alice has no balance.
        {
            assert!(funcs::balance_of(&map, alice()).is_zero());
            assert_eq!(funcs::balance_of(&map, bob()), U256::exp10(42));

            assert_err!(
                funcs::transfer(&mut map, alice(), bob(), U256::exp10(20)),
                Error::InsufficientBalance
            );

            assert!(funcs::balance_of(&map, alice()).is_zero());
            assert_eq!(funcs::balance_of(&map, bob()), U256::exp10(42));
        }

        // # Test case #2.
        // Bob transfers to Alice, when Bob's balance is less than required.
        {
            assert!(funcs::balance_of(&map, alice()).is_zero());
            assert_eq!(funcs::balance_of(&map, bob()), U256::exp10(42));

            assert_err!(
                funcs::transfer(&mut map, bob(), alice(), U256::exp10(50)),
                Error::InsufficientBalance
            );

            assert!(funcs::balance_of(&map, alice()).is_zero());
            assert_eq!(funcs::balance_of(&map, bob()), U256::exp10(42));
        }

        // # Test case #3.
        // Dave transfers to Bob, causing numeric overflow.
        {
            assert_eq!(funcs::balance_of(&map, bob()), U256::exp10(42));
            assert_eq!(funcs::balance_of(&map, dave()), U256::MAX);

            assert_err!(
                funcs::transfer(&mut map, dave(), bob(), U256::MAX),
                Error::NumericOverflow
            );

            assert_eq!(funcs::balance_of(&map, bob()), U256::exp10(42));
            assert_eq!(funcs::balance_of(&map, dave()), U256::MAX);
        }

        // # Test case #4.
        // Bob transfers to Alice, when Alice's account doesn't exist.
        {
            assert!(funcs::balance_of(&map, alice()).is_zero());
            assert_eq!(funcs::balance_of(&map, bob()), U256::exp10(42));

            assert_ok!(
                funcs::transfer(&mut map, bob(), alice(), U256::exp10(10)),
                true
            );

            assert_eq!(funcs::balance_of(&map, alice()), U256::exp10(10));
            assert_eq!(
                funcs::balance_of(&map, bob()),
                U256::exp10(42) - U256::exp10(10)
            );
        }

        // # Test case #5.
        // Bob transfers to Alice, when Alice's account exists.
        {
            assert_eq!(funcs::balance_of(&map, alice()), U256::exp10(10));
            assert_eq!(
                funcs::balance_of(&map, bob()),
                U256::exp10(42) - U256::exp10(10)
            );

            assert_ok!(
                funcs::transfer(&mut map, bob(), alice(), U256::exp10(10)),
                true
            );

            assert_eq!(
                funcs::balance_of(&map, alice()),
                U256::exp10(10).saturating_mul(2.into())
            );
            assert_eq!(
                funcs::balance_of(&map, bob()),
                U256::exp10(42) - U256::exp10(10).saturating_mul(2.into())
            );
        }

        // # Test case #6.
        // Bob transfers to Alice, when Alice's account exists and Bob's is removed.
        {
            assert_eq!(
                funcs::balance_of(&map, alice()),
                U256::exp10(10).saturating_mul(2.into())
            );
            assert_eq!(
                funcs::balance_of(&map, bob()),
                U256::exp10(42) - U256::exp10(10).saturating_mul(2.into())
            );

            assert_ok!(
                funcs::transfer(
                    &mut map,
                    bob(),
                    alice(),
                    U256::exp10(42) - U256::exp10(10).saturating_mul(2.into())
                ),
                true
            );

            assert_eq!(funcs::balance_of(&map, alice()), U256::exp10(42));
            assert!(funcs::balance_of(&map, bob()).is_zero());
        }

        // # Test case #7.
        // Alice transfers to Charlie, when Alice's account is removed and Charlie's is created.
        {
            assert_eq!(funcs::balance_of(&map, alice()), U256::exp10(42));
            assert!(funcs::balance_of(&map, charlie()).is_zero());

            assert_ok!(
                funcs::transfer(&mut map, alice(), charlie(), U256::exp10(42)),
                true
            );

            assert!(funcs::balance_of(&map, alice()).is_zero());
            assert_eq!(funcs::balance_of(&map, charlie()), U256::exp10(42));
        }

        // # Test case #8.
        // Transfer is always noop when from == to.
        {
            assert!(funcs::balance_of(&map, alice()).is_zero());
            assert_ok!(
                funcs::transfer(&mut map, alice(), alice(), U256::exp10(42)),
                false
            );
            assert!(funcs::balance_of(&map, alice()).is_zero());

            assert_eq!(funcs::balance_of(&map, charlie()), U256::exp10(42));
            assert_ok!(
                funcs::transfer(&mut map, charlie(), charlie(), U256::exp10(42)),
                false
            );
            assert_eq!(funcs::balance_of(&map, charlie()), U256::exp10(42));
        }

        // # Test case #9.
        // Transfer is always noop when value is zero.
        {
            assert!(funcs::balance_of(&map, alice()).is_zero());
            assert_eq!(funcs::balance_of(&map, charlie()), U256::exp10(42));

            assert_ok!(
                funcs::transfer(&mut map, alice(), charlie(), U256::zero()),
                false
            );
            assert!(funcs::balance_of(&map, alice()).is_zero());
            assert_eq!(funcs::balance_of(&map, charlie()), U256::exp10(42));

            assert_ok!(
                funcs::transfer(&mut map, charlie(), alice(), U256::zero()),
                false
            );
            assert!(funcs::balance_of(&map, alice()).is_zero());
            assert_eq!(funcs::balance_of(&map, charlie()), U256::exp10(42));
        }
    }

    // Since this uses [`super::transfer`] in underlying impl, it needs only
    // check approval specific logic and few transfer's happy cases.
    #[test]
    fn transfer_from() {
        // Initializing thread logger.
        let _ = env_logger::try_init();

        // Creating empty allowances map.
        let mut amap = allowances_map([]);

        // Creating balances map with two equal balances belonged to Bob and Dave.
        let mut bmap = balances_map([(bob(), U256::exp10(42)), (dave(), U256::exp10(42))]);

        // # Test case #1.
        // Bob doesn't need approve to transfer from self to Alice.
        // With zero value nothing's changed.
        {
            assert_ok!(
                funcs::transfer_from(&mut amap, &mut bmap, bob(), bob(), alice(), U256::zero()),
                false
            );
            assert!(funcs::balance_of(&bmap, alice()).is_zero());
            assert_eq!(funcs::balance_of(&bmap, bob()), U256::exp10(42));
        }

        // # Test case #2.
        // Bob doesn't need approve to transfer from self to Alice.
        {
            assert_ok!(
                funcs::transfer_from(&mut amap, &mut bmap, bob(), bob(), alice(), U256::exp10(42)),
                true
            );
            assert_eq!(funcs::balance_of(&bmap, alice()), U256::exp10(42));
            assert!(funcs::balance_of(&bmap, bob()).is_zero());
        }

        // # Test case #3.
        // Noop on self transfer with self approve.
        {
            assert!(funcs::balance_of(&bmap, bob()).is_zero());
            assert_ok!(
                funcs::transfer_from(&mut amap, &mut bmap, bob(), bob(), bob(), U256::exp10(42)),
                false
            );
            assert!(funcs::balance_of(&bmap, bob()).is_zero());

            assert_eq!(funcs::balance_of(&bmap, alice()), U256::exp10(42));
            assert_ok!(
                funcs::transfer_from(
                    &mut amap,
                    &mut bmap,
                    alice(),
                    alice(),
                    alice(),
                    U256::exp10(42)
                ),
                false
            );
            assert_eq!(funcs::balance_of(&bmap, alice()), U256::exp10(42));
        }

        // # Test case #4.
        // Bob tries to perform transfer from Alice to Charlie with no approval exists.
        {
            assert_eq!(funcs::balance_of(&bmap, alice()), U256::exp10(42));
            assert!(funcs::balance_of(&bmap, bob()).is_zero());
            assert!(funcs::balance_of(&bmap, charlie()).is_zero());

            assert_err!(
                funcs::transfer_from(
                    &mut amap,
                    &mut bmap,
                    bob(),
                    alice(),
                    charlie(),
                    U256::exp10(20)
                ),
                Error::InsufficientAllowance,
            );

            assert_eq!(funcs::balance_of(&bmap, alice()), U256::exp10(42));
            assert!(funcs::balance_of(&bmap, bob()).is_zero());
            assert!(funcs::balance_of(&bmap, charlie()).is_zero());
        }

        // # Test case #5.
        // Bob tries to perform transfer from Alice to Charlie with insufficient approval.
        {
            assert_eq!(funcs::balance_of(&bmap, alice()), U256::exp10(42));
            assert!(funcs::balance_of(&bmap, bob()).is_zero());
            assert!(funcs::balance_of(&bmap, charlie()).is_zero());

            assert!(funcs::approve(&mut amap, alice(), bob(), U256::exp10(19)));

            assert_err!(
                funcs::transfer_from(
                    &mut amap,
                    &mut bmap,
                    bob(),
                    alice(),
                    charlie(),
                    U256::exp10(20)
                ),
                Error::InsufficientAllowance,
            );

            assert_eq!(funcs::balance_of(&bmap, alice()), U256::exp10(42));
            assert!(funcs::balance_of(&bmap, bob()).is_zero());
            assert!(funcs::balance_of(&bmap, charlie()).is_zero());
            assert_eq!(funcs::allowance(&amap, alice(), bob()), U256::exp10(19));
        }

        // # Test case #6.
        // Bob tries to perform transfer from Alice to Charlie with insufficient balance.
        {
            assert!(funcs::approve(&mut amap, alice(), bob(), U256::MAX));

            assert_err!(
                funcs::transfer_from(
                    &mut amap,
                    &mut bmap,
                    bob(),
                    alice(),
                    charlie(),
                    U256::exp10(43)
                ),
                Error::InsufficientBalance,
            );

            assert_eq!(funcs::balance_of(&bmap, alice()), U256::exp10(42));
            assert!(funcs::balance_of(&bmap, bob()).is_zero());
            assert!(funcs::balance_of(&bmap, charlie()).is_zero());
        }

        // * `Ok(true)` when allowance is changed
        // * `Ok(true)` when allowance is removed

        // # Test case #7.
        // Bob performs transfer from Alice to Charlie and allowance is changed.
        {
            assert_ok!(
                funcs::transfer_from(
                    &mut amap,
                    &mut bmap,
                    bob(),
                    alice(),
                    charlie(),
                    U256::exp10(42)
                ),
                true
            );

            assert!(funcs::balance_of(&bmap, alice()).is_zero());
            assert!(funcs::balance_of(&bmap, bob()).is_zero());
            assert_eq!(funcs::balance_of(&bmap, charlie()), U256::exp10(42));
            assert_eq!(
                funcs::allowance(&amap, alice(), bob()),
                U256::MAX - U256::exp10(42)
            );
        }

        // # Test case #8.
        // Alice performs transfer from Charlie to Dave and allowance is removed.
        {
            assert!(funcs::approve(
                &mut amap,
                charlie(),
                alice(),
                U256::exp10(42)
            ));

            assert_ok!(
                funcs::transfer_from(
                    &mut amap,
                    &mut bmap,
                    alice(),
                    charlie(),
                    dave(),
                    U256::exp10(42)
                ),
                true
            );

            assert!(funcs::balance_of(&bmap, alice()).is_zero());
            assert!(funcs::balance_of(&bmap, bob()).is_zero());
            assert!(funcs::balance_of(&bmap, charlie()).is_zero());
            assert_eq!(
                funcs::balance_of(&bmap, dave()),
                U256::exp10(42).saturating_mul(2.into())
            );
            assert!(funcs::allowance(&amap, charlie(), alice()).is_zero());
        }
    }

    mod utils {
        use super::*;

        pub fn allowances_map<const N: usize>(
            content: [(ActorId, ActorId, U256); N],
        ) -> AllowancesMap {
            content
                .into_iter()
                .map(|(k1, k2, v)| ((k1, k2), v))
                .collect()
        }

        pub fn balances_map<const N: usize>(content: [(ActorId, U256); N]) -> BalancesMap {
            content.into_iter().map(|(k, v)| (k, v)).collect()
        }

        pub fn alice() -> ActorId {
            1u64.into()
        }

        pub fn bob() -> ActorId {
            2u64.into()
        }

        pub fn charlie() -> ActorId {
            3u64.into()
        }

        pub fn dave() -> ActorId {
            4u64.into()
        }
    }
}
