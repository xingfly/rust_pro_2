#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;
use sp_io::offchain::random_seed;

#[frame_support::pallet]
pub mod pallet {
	use frame_support::{
		dispatch::{DispatchResult, DispatchResultWithPostInfo},
		ensure,
		pallet_prelude::*,
		sp_runtime::traits::{Hash, Zero},
		traits::{Currency, ExistenceRequirement, Randomness, ReservableCurrency},
	};
	use frame_system::{ensure_signed, pallet, pallet_prelude::*};
	use scale_info::TypeInfo;
	use sp_io::hashing::blake2_128;

	#[derive(Encode, Decode, TypeInfo)]
	pub struct Kitty {
		pub dna: [u8; 16],
	}

	type BalanceOf<T> =
		<<T as Config>::Currency as Currency<<T as frame_system::Config>::AccountId>>::Balance;

	#[pallet::storage]
	#[pallet::getter(fn kitties_count)]
	pub(super) type KittiesCount<T> = StorageValue<_, u32>;

	#[pallet::storage]
	#[pallet::getter(fn kitties)]
	pub type Kitties<T> = StorageMap<_, Blake2_128Concat, KittyIndex, Option<Kitty>, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn owner)]
	pub type Owner<T: Config> =
		StorageMap<_, Blake2_128Concat, KittyIndex, Option<T::AccountId>, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn kitties_list_for_sales)]
	pub type ListForSale<T: Config> =
		StorageMap<_, Blake2_128Concat, KittyIndex, Option<BalanceOf<T>>, ValueQuery>;

	type KittyIndex = u32;

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config {
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;
		type Randomness: Randomness<Self::Hash, Self::BlockNumber>;
		type Currency: Currency<Self::AccountId> + ReservableCurrency<Self::AccountId>;
		#[pallet::constant]
		type StakeForEachKitty: Get<BalanceOf<Self>>;
	}

	// Errors.
	#[pallet::error]
	pub enum Error<T> {
		KittiesCountOverflow,
		NotOwner,
		SameParentIndex,
		InvalidKittyIndex,
		BuyerIsOwner,
		KittyNotForSell,
		NotEnoughBalanceForBuying,
		NotEnoughBalanceForStaking,
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		KittyCreate(T::AccountId, KittyIndex),
		KittyTransfer(T::AccountId, T::AccountId, KittyIndex),
		KittyListed(T::AccountId, KittyIndex, Option<BalanceOf<T>>),
		KittySold(T::AccountId, T::AccountId, KittyIndex),
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		// 创建
		#[pallet::weight(0)]
		pub fn create(origin: OriginFor<T>) -> DispatchResult {
			let who = ensure_signed(origin)?;
			// 随机生成DNA
			let dna = Self::random_value(&who);
			// 创建+质押Kitty
			Self::create_kitty_with_stake(&who, dna)
		}

		// 繁殖
		#[pallet::weight(0)]
		pub fn breed(
			origin: OriginFor<T>,
			kitty_id_1: KittyIndex,
			kitty_id_2: KittyIndex,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;
			// 繁殖不能是同一个Kitty
			ensure!(kitty_id_1 != kitty_id_2, Error::<T>::SameParentIndex);
			// 获取Kitty1
			let kitty1 = Self::kitties(kitty_id_1).ok_or(Error::<T>::InvalidKittyIndex)?;
			// 获取Kitty2
			let kitty2 = Self::kitties(kitty_id_2).ok_or(Error::<T>::InvalidKittyIndex)?;

			// 获取Parents Kitty的DNA
			let dna_1 = kitty1.dna;
			let dna_2 = kitty2.dna;
			// 混淆DNA
			let selector = Self::random_value(&who);
			let mut new_dna = [0u8; 16];
			for i in 0..dna_1.len() {
				new_dna[i] = (selector[i] & dna_1[i]) | (!selector[i] & dna_2[i]);
			}
			// 质押+创建Kitty
			Self::create_kitty_with_stake(&who, new_dna)
		}

		// 卖出
		#[pallet::weight(0)]
		pub fn sell(
			origin: OriginFor<T>,
			kitty_id: KittyIndex,
			price: Option<BalanceOf<T>>,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;
			// 检查是否有权限卖出
			ensure!(Some(who.clone()) == Owner::<T>::get(kitty_id), Error::<T>::NotOwner);
			// 将Kitty添加到出售列表
			ListForSale::<T>::insert(kitty_id, price);
			// 发出Kitty卖出事件
			Self::deposit_event(Event::KittyListed(who, kitty_id, price));
			Ok(())
		}

		// 转移
		#[pallet::weight(0)]
		pub fn transfer(
			origin: OriginFor<T>,
			new_owner: T::AccountId,
			kitty_id: KittyIndex,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;
			// 检查是否是原拥有者
			ensure!(Some(who.clone()) == Owner::<T>::get(kitty_id), Error::<T>::NotOwner);
			// 更新Kitty的拥有者（双方分别释放和重新质押）
			if let Ok(_) = Self::conversion_stake(&who, &new_owner) {
				// 更新Kitty的所有者为新的拥有者
				Owner::<T>::insert(kitty_id, Some(new_owner.clone()));
				// 发布转移事件
				Self::deposit_event(Event::KittyTransfer(who, new_owner, kitty_id));
			}
			Ok(())
		}

		// 购买
		#[pallet::weight(0)]
		pub fn buy(origin: OriginFor<T>, kitty_id: KittyIndex) -> DispatchResult {
			let buyer = ensure_signed(origin)?;
			// 获取Kitty的所有者
			let seller = Owner::<T>::get(kitty_id).unwrap();
			// 检查购买者和所有者是否是同一个人
			ensure!(Some(buyer.clone()) != Some(seller.clone()), Error::<T>::BuyerIsOwner);
			// 获取Kitty的价格，如果不存在表示Kitty不出售
			let kitty_price = ListForSale::<T>::get(kitty_id).ok_or(Error::<T>::KittyNotForSell)?;
			// 获取买家余额
			let buyer_balance = T::Currency::free_balance(&buyer);
			// 质押的金额
			let stake_amount = T::StakeForEachKitty::get();
			// 检查买家余额是否足够
			ensure!(
				buyer_balance > (kitty_price + stake_amount),
				Error::<T>::NotEnoughBalanceForBuying
			);
			if let Ok(_) = Self::conversion_stake(&seller, &buyer) {
				// 买家向卖家转账
				T::Currency::transfer(
					&buyer,
					&seller,
					kitty_price,
					ExistenceRequirement::KeepAlive,
				)?;
				// 更新Kitty的所有者为买家
				Owner::<T>::insert(kitty_id, Some(buyer.clone()));
				// 将Kitty从出售列表中移除
				ListForSale::<T>::remove(kitty_id);
				// 发出交易完成事件
				Self::deposit_event(Event::KittySold(buyer, seller, kitty_id));
			}
			Ok(())
		}
	}

	impl<T: Config> Pallet<T> {
		fn random_value(sender: &T::AccountId) -> [u8; 16] {
			let payload = (
				T::Randomness::random_seed(),
				&sender,
				<frame_system::Pallet<T>>::extrinsic_index(),
			);
			payload.using_encoded(blake2_128)
		}

		fn conversion_stake(
			old_owner: &T::AccountId,
			new_owner: &T::AccountId,
		) -> Result<(), &'static str> {
			// 获取质押金额
			let stake_amount = T::StakeForEachKitty::get();
			// 质押新的拥有者一定金额
			T::Currency::reserve(new_owner, stake_amount)
				.map_err(|_| Error::<T>::NotEnoughBalanceForStaking)?;
			// 解除旧拥有者的质押
			T::Currency::unreserve(old_owner, stake_amount);
			Ok(())
		}

		fn create_kitty_with_stake(owner: &T::AccountId, dna: [u8; 16]) -> DispatchResult {
			// Child Kitty的ID
			let kitty_id = match Self::kitties_count() {
				Some(id) => {
					ensure!(id != KittyIndex::max_value(), Error::<T>::KittiesCountOverflow);
					id
				}
				None => 1,
			};
			// 获取质押的金额
			let stake_amount = T::StakeForEachKitty::get();
			// 质押创建者一定的金额
			T::Currency::reserve(&owner, stake_amount)
				.map_err(|_| Error::<T>::NotEnoughBalanceForStaking)?;
			// 将Kitty加入Kitties集合
			Kitties::<T>::insert(kitty_id, Some(Kitty { dna }));
			// 为Kitty绑定所有人
			Owner::<T>::insert(kitty_id, Some(owner.clone()));
			// 更新下一个Kitty的ID
			KittiesCount::<T>::put(kitty_id + 1);
			// 发出创建事件
			Self::deposit_event(Event::KittyCreate(owner.clone(), kitty_id));
			Ok(())
		}
	}
}
