//! Demo target for ICP Code Mode: a small store with 10 methods (7 reads, 3 writes)
//! and deterministic seed data. It exists so the demo can show one script replacing
//! many single-method tool calls.
//!
//! State is heap-only and reseeded on every install/upgrade — it's demo data.
//! There is no access control: anyone can call the writes. Deploy it on a local network
//! only.

use candid::{CandidType, Deserialize, Nat, Principal};
use std::cell::RefCell;

#[derive(CandidType, Deserialize, Clone)]
pub struct Product {
    pub id: u64,
    pub name: String,
    pub category: String,
    pub price_cents: Nat,
}

#[derive(CandidType, Deserialize, Clone, Copy, PartialEq)]
pub enum Tier {
    Bronze,
    Silver,
    Gold,
}

#[derive(CandidType, Deserialize, Clone)]
pub struct Customer {
    pub id: Principal,
    pub name: String,
    pub email: String,
    pub tier: Tier,
    pub joined_at: u64,
}

#[derive(CandidType, Deserialize, Clone)]
pub struct OrderItem {
    pub product_id: u64,
    pub quantity: u32,
}

#[derive(CandidType, Deserialize, Clone, Copy, PartialEq)]
pub enum OrderStatus {
    Pending,
    Shipped,
    Delivered,
    Cancelled,
}

#[derive(CandidType, Deserialize, Clone)]
pub struct Order {
    pub id: u64,
    pub customer: Principal,
    pub items: Vec<OrderItem>,
    pub total_cents: Nat,
    pub status: OrderStatus,
    pub created_at: u64,
}

#[derive(CandidType, Deserialize, Clone)]
pub struct Inventory {
    pub product_id: u64,
    pub on_hand: u32,
    /// Units held by pending orders.
    pub reserved: u32,
    pub reorder_level: u32,
}

#[derive(CandidType, Deserialize, Clone)]
pub struct CategorySales {
    pub category: String,
    pub units: u64,
    pub revenue_cents: Nat,
}

#[derive(CandidType, Deserialize, Clone)]
pub struct SalesSummary {
    pub orders: u64,
    pub revenue_cents: Nat,
    pub by_category: Vec<CategorySales>,
}

#[derive(CandidType, Deserialize, Clone)]
pub enum OrderError {
    UnknownCustomer,
    EmptyOrder,
    UnknownProduct(u64),
    OutOfStock { product_id: u64, available: u32 },
    ZeroQuantity(u64),
    /// More than `MAX_LINES` lines in one order.
    TooManyLines(u32),
    /// The store already holds `MAX_ORDERS` orders.
    StoreFull,
}

/// Bounds that keep `list_orders(null)` far below the 3 MiB reply limit
/// (worst case about 1,000 x 300 bytes).
const MAX_LINES: usize = 20;
const MAX_ORDERS: usize = 1_000;

struct Store {
    products: Vec<(Product, u64)>, // (product, price in cents as u64 for arithmetic)
    customers: Vec<Customer>,
    orders: Vec<Order>,
    inventory: Vec<Inventory>,
}

const CATALOG: [(&str, &str, u64); 12] = [
    ("Ethiopia Yirgacheffe beans 1kg", "coffee", 3400),
    ("Colombia Huila beans 1kg", "coffee", 2900),
    ("Espresso blend 500g", "coffee", 1800),
    ("Decaf Swiss water 500g", "coffee", 1900),
    ("Sencha green tea 100g", "tea", 1200),
    ("Assam breakfast tea 250g", "tea", 1100),
    ("Jasmine pearls 100g", "tea", 2200),
    ("Rooibos vanilla 200g", "tea", 950),
    ("Ceramic pour-over dripper", "kitchen", 2600),
    ("Gooseneck kettle", "kitchen", 6400),
    ("Burr grinder", "kitchen", 12900),
    ("Stoneware mug", "kitchen", 1500),
];

const CUSTOMERS: [(&str, Tier); 8] = [
    ("Ada Okafor", Tier::Gold),
    ("Bruno Silva", Tier::Silver),
    ("Chen Wei", Tier::Bronze),
    ("Dana Levi", Tier::Gold),
    ("Emeka Nwosu", Tier::Bronze),
    ("Freya Lund", Tier::Silver),
    ("Gita Rao", Tier::Bronze),
    ("Hugo Martin", Tier::Silver),
];

const DAY_NS: u64 = 86_400 * 1_000_000_000;
const EPOCH: u64 = 1_767_225_600 * 1_000_000_000; // 2026-01-01

fn customer_principal(i: usize) -> Principal {
    Principal::from_slice(&[0xC0, 0xFF, 0xEE, i as u8, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06])
}

/// Deterministic seed: 12 products, 8 customers, 60 orders.
fn seed() -> Store {
    let products: Vec<(Product, u64)> = CATALOG
        .iter()
        .enumerate()
        .map(|(i, (name, cat, cents))| {
            let p = Product { id: i as u64 + 1, name: (*name).into(), category: (*cat).into(), price_cents: Nat::from(*cents) };
            (p, *cents)
        })
        .collect();
    let customers: Vec<Customer> = CUSTOMERS
        .iter()
        .enumerate()
        .map(|(i, (name, tier))| Customer {
            id: customer_principal(i),
            name: (*name).into(),
            email: format!("{}@example.com", name.split(' ').next().unwrap_or("x").to_lowercase()),
            tier: *tier,
            joined_at: EPOCH - (i as u64 + 1) * 30 * DAY_NS,
        })
        .collect();
    let mut orders = vec![];
    for k in 0..60u64 {
        let customer = customers[((k * 5 + 3) % 8) as usize].id;
        let items: Vec<OrderItem> = (0..1 + k % 3)
            .map(|i| OrderItem { product_id: (k * 7 + i * 3) % 12 + 1, quantity: (1 + (k + i) % 3) as u32 })
            .collect();
        let total: u64 = items.iter().map(|it| products[(it.product_id - 1) as usize].1 * it.quantity as u64).sum();
        let status = match k % 10 {
            0..=5 => OrderStatus::Delivered,
            6 | 7 => OrderStatus::Shipped,
            8 => OrderStatus::Pending,
            _ => OrderStatus::Cancelled,
        };
        orders.push(Order { id: k + 1, customer, items, total_cents: Nat::from(total), status, created_at: EPOCH + k * DAY_NS });
    }
    let inventory = (1..=12u64)
        .map(|id| {
            let reserved = orders
                .iter()
                .filter(|o| o.status == OrderStatus::Pending)
                .flat_map(|o| &o.items)
                .filter(|it| it.product_id == id)
                .map(|it| it.quantity)
                .sum();
            // Never oversold: on hand covers what pending orders reserve.
            Inventory { product_id: id, on_hand: reserved + ((id * 13 + 7) % 25) as u32, reserved, reorder_level: 10 }
        })
        .collect();
    Store { products, customers, orders, inventory }
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(seed());
}

fn with<R>(f: impl FnOnce(&Store) -> R) -> R {
    STORE.with(|s| f(&s.borrow()))
}

fn with_mut<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    STORE.with(|s| f(&mut s.borrow_mut()))
}

// ---------- reads ----------

/// All products, optionally only one category ("coffee", "tea", "kitchen").
#[ic_cdk::query]
fn list_products(category: Option<String>) -> Vec<Product> {
    with(|s| {
        s.products
            .iter()
            .filter(|(p, _)| category.as_ref().is_none_or(|c| p.category.eq_ignore_ascii_case(c)))
            .map(|(p, _)| p.clone())
            .collect()
    })
}

#[ic_cdk::query]
fn get_product(id: u64) -> Option<Product> {
    with(|s| s.products.iter().find(|(p, _)| p.id == id).map(|(p, _)| p.clone()))
}

/// Case-insensitive substring match on name or category.
#[ic_cdk::query]
fn search_products(text: String) -> Vec<Product> {
    let q = text.to_lowercase();
    with(|s| {
        s.products
            .iter()
            .filter(|(p, _)| p.name.to_lowercase().contains(&q) || p.category.to_lowercase().contains(&q))
            .map(|(p, _)| p.clone())
            .collect()
    })
}

#[ic_cdk::query]
fn get_inventory(product_id: u64) -> Option<Inventory> {
    with(|s| s.inventory.iter().find(|i| i.product_id == product_id).cloned())
}

#[ic_cdk::query]
fn get_customer(id: Principal) -> Option<Customer> {
    with(|s| s.customers.iter().find(|c| c.id == id).cloned())
}

/// All orders, optionally for one customer, oldest first.
#[ic_cdk::query]
fn list_orders(customer: Option<Principal>) -> Vec<Order> {
    with(|s| s.orders.iter().filter(|o| customer.is_none_or(|c| o.customer == c)).cloned().collect())
}

/// Revenue over all non-cancelled orders, total and per category.
#[ic_cdk::query]
fn sales_summary() -> SalesSummary {
    with(|s| {
        let mut by: Vec<(String, u64, u128)> = vec![];
        let mut revenue = 0u128;
        let mut count = 0u64;
        for o in s.orders.iter().filter(|o| o.status != OrderStatus::Cancelled) {
            count += 1;
            for it in &o.items {
                let (p, cents) = &s.products[(it.product_id - 1) as usize];
                let line = *cents as u128 * it.quantity as u128;
                revenue += line;
                match by.iter_mut().find(|(c, ..)| *c == p.category) {
                    Some(e) => {
                        e.1 += it.quantity as u64;
                        e.2 += line;
                    }
                    None => by.push((p.category.clone(), it.quantity as u64, line)),
                }
            }
        }
        SalesSummary {
            orders: count,
            revenue_cents: Nat::from(revenue),
            by_category: by
                .into_iter()
                .map(|(category, units, rev)| CategorySales { category, units, revenue_cents: Nat::from(rev) })
                .collect(),
        }
    })
}

// ---------- writes ----------

/// Place a pending order; reserves stock.
#[ic_cdk::update]
fn create_order(customer: Principal, items: Vec<OrderItem>) -> Result<Order, OrderError> {
    with_mut(|s| {
        if items.is_empty() {
            return Err(OrderError::EmptyOrder);
        }
        if items.len() > MAX_LINES {
            return Err(OrderError::TooManyLines(MAX_LINES as u32));
        }
        if s.orders.len() >= MAX_ORDERS {
            return Err(OrderError::StoreFull);
        }
        if !s.customers.iter().any(|c| c.id == customer) {
            return Err(OrderError::UnknownCustomer);
        }
        let mut total = 0u128;
        // Quantity per product across all lines, so repeating a product can't oversell it.
        let mut wanted: Vec<(u64, u64)> = vec![];
        for it in &items {
            let (_, cents) = s.products.iter().find(|(p, _)| p.id == it.product_id).ok_or(OrderError::UnknownProduct(it.product_id))?;
            if it.quantity == 0 {
                return Err(OrderError::ZeroQuantity(it.product_id));
            }
            let n = match wanted.iter_mut().find(|(id, _)| *id == it.product_id) {
                Some(w) => {
                    w.1 += it.quantity as u64;
                    w.1
                }
                None => {
                    wanted.push((it.product_id, it.quantity as u64));
                    it.quantity as u64
                }
            };
            let inv = &s.inventory[(it.product_id - 1) as usize];
            let available = inv.on_hand.saturating_sub(inv.reserved);
            if n > available as u64 {
                return Err(OrderError::OutOfStock { product_id: it.product_id, available });
            }
            total += *cents as u128 * it.quantity as u128;
        }
        for it in &items {
            s.inventory[(it.product_id - 1) as usize].reserved += it.quantity;
        }
        let order = Order {
            id: s.orders.len() as u64 + 1,
            customer,
            items,
            total_cents: Nat::from(total),
            status: OrderStatus::Pending,
            created_at: ic_cdk::api::time(),
        };
        s.orders.push(order.clone());
        Ok(order)
    })
}

/// Cancel a pending order and release its reserved stock.
#[ic_cdk::update]
fn cancel_order(id: u64) -> Result<Order, String> {
    with_mut(|s| {
        let o = s.orders.iter_mut().find(|o| o.id == id).ok_or("no such order")?;
        if o.status != OrderStatus::Pending {
            return Err("only pending orders can be cancelled".into());
        }
        o.status = OrderStatus::Cancelled;
        let o = o.clone();
        for it in &o.items {
            let inv = &mut s.inventory[(it.product_id - 1) as usize];
            inv.reserved = inv.reserved.saturating_sub(it.quantity);
        }
        Ok(o)
    })
}

#[ic_cdk::update]
fn restock(product_id: u64, quantity: u32) -> Result<Inventory, String> {
    with_mut(|s| {
        let inv = s.inventory.iter_mut().find(|i| i.product_id == product_id).ok_or("no such product")?;
        inv.on_hand = inv.on_hand.checked_add(quantity).ok_or("stock would overflow")?;
        Ok(inv.clone())
    })
}

/// Lets callers (e.g. ICP Code Mode) read this interface where `canister_metadata` is not
/// available to them.
#[ic_cdk::query(hidden = true)]
fn __get_candid_interface_tmp_hack() -> String {
    __export_service()
}

ic_cdk::export_candid!();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn did_file_matches_exported_interface() {
        let actual = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/demo_store.did")).unwrap();
        assert_eq!(actual.trim(), __export_service().trim(), "regenerate demo_store.did");
    }

    #[test]
    fn seed_is_consistent() {
        let s = seed();
        assert_eq!((s.products.len(), s.customers.len(), s.orders.len()), (12, 8, 60));
        assert!(s.inventory.iter().any(|i| i.on_hand - i.reserved < i.reorder_level), "some products need restocking");
        assert!(s.inventory.iter().all(|i| i.on_hand >= i.reserved), "nothing oversold");
        assert!(s.orders.iter().all(|o| !o.items.is_empty()));
        assert_eq!(list_products(Some("TEA".into())).len(), 4);
        assert_eq!(search_products("mug".into()).len(), 1);
        assert_eq!(sales_summary().by_category.len(), 3);
    }

    #[test]
    fn create_order_rejects_duplicate_product_oversell_and_zero_quantity() {
        let customer = customer_principal(0);
        let inv = get_inventory(1).unwrap();
        let avail = inv.on_hand - inv.reserved;
        assert!(avail > 0);
        let dup = vec![OrderItem { product_id: 1, quantity: avail }, OrderItem { product_id: 1, quantity: avail }];
        assert!(create_order(customer, dup).is_err());
        assert!(create_order(customer, vec![OrderItem { product_id: 1, quantity: 0 }]).is_err());
    }

    #[test]
    fn read_error_and_write_edge_cases() {
        let cust = customer_principal(0);
        let stranger = Principal::from_text("aaaaa-aa").unwrap();

        assert!(get_product(1).is_some() && get_product(999).is_none());
        assert!(get_inventory(1).is_some() && get_inventory(999).is_none());
        assert!(get_customer(cust).is_some() && get_customer(stranger).is_none());
        assert_eq!(list_products(None).len(), 12);
        assert!(list_products(Some("unknown".into())).is_empty());
        assert_eq!(list_orders(None).len(), 60);
        assert!(!list_orders(Some(cust)).is_empty());
        assert!(list_orders(Some(stranger)).is_empty());
        assert!(!search_products("BEANS".into()).is_empty());
        assert!(search_products("nonexistent".into()).is_empty());

        // Order error branches (none touch ic_cdk::api::time).
        assert!(matches!(create_order(cust, vec![]), Err(OrderError::EmptyOrder)));
        assert!(matches!(create_order(stranger, vec![OrderItem { product_id: 1, quantity: 1 }]), Err(OrderError::UnknownCustomer)));
        assert!(matches!(create_order(cust, vec![OrderItem { product_id: 999, quantity: 1 }]), Err(OrderError::UnknownProduct(999))));
        assert!(matches!(create_order(cust, vec![OrderItem { product_id: 1, quantity: u32::MAX }]), Err(OrderError::OutOfStock { product_id: 1, .. })));

        // Cancel order edge cases & seeded pending order (#9, since k=8 => 8 % 10 == 8).
        assert!(cancel_order(9999).is_err());
        assert!(cancel_order(1).is_err()); // Delivered
        let cancelled = cancel_order(9).unwrap();
        assert!(cancelled.status == OrderStatus::Cancelled);

        // Restock hit & miss.
        assert!(restock(999, 5).is_err());
        let before = get_inventory(2).unwrap().on_hand;
        assert_eq!(restock(2, 5).unwrap().on_hand, before + 5);
        // Finding 148: overflow is an error, not a silent saturation.
        assert!(restock(2, u32::MAX).is_err());
        assert_eq!(get_inventory(2).unwrap().on_hand, before + 5);
    }

    #[test]
    fn orders_are_bounded() {
        let cust = customer_principal(0);
        let lines = vec![OrderItem { product_id: 1, quantity: 1 }; MAX_LINES + 1];
        assert!(matches!(create_order(cust, lines), Err(OrderError::TooManyLines(_))));
        with_mut(|s| {
            let o = s.orders[0].clone();
            s.orders.resize(MAX_ORDERS, o);
        });
        assert!(matches!(create_order(cust, vec![OrderItem { product_id: 1, quantity: 1 }]), Err(OrderError::StoreFull)));
    }
}
