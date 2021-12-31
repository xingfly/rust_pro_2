use crate::mock::{new_test_ext, Event as TestEvent, Origin, SubstrateKitties, System, Test};
use frame_support::{assert_noop, assert_ok};
use super::*;

#[test]
fn create_works() {
	new_test_ext().execute_with(|| {
		let accound_id: u64 = 1;
        assert_ok!(SubstrateKitties::create(Origin::signed(accound_id)));
	});
}
