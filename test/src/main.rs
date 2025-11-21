use forgen_test::{distance, midpoint, Point};
use serde::{Deserialize, Serialize};

const PI: f64 = 3.1415;
const MAX_USERS: u32 = 100;

type UserId = u64;
type Score = f32;

#[derive(Serialize, Deserialize, Debug)]
pub struct User {
    pub id: UserId,
    pub name: String,
    score: Score,
}

#[derive(Serialize, Deserialize, Debug)]
struct Admin {
    pub id: UserId,
    level: u8,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Role {
    Guest,
    Member,
    Admin(Admin),
}

pub trait Greet {
    fn greet(&self) -> String;
}

impl Greet for User {
    fn greet(&self) -> String {
        format!("Hello, {}!", self.name)
    }
}

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn private_helper(x: f64) -> f64 {
    x * PI
}

pub static mut GLOBAL_COUNT: u32 = 0;

fn main() {
    {
        let mut temp_score: Score = 0.0;
        let user_id: UserId = 42;
    }

    let alice = User {
        id: 1,
        name: String::from("Alice"),
        score: 10.0,
    };

    // Test serde serialization
    let json = serde_json::to_string(&alice).unwrap();
    println!("Serialized: {}", json);

    let deserialized: User = serde_json::from_str(&json).unwrap();
    println!("Deserialized: {:?}", deserialized);

    let greeting = alice.greet();
    println!("{}", greeting);

    let sum = add(5, 7);
    println!("5 + 7 = {}", sum);

    // Test using lib module
    let p1 = Point { x: 0.0, y: 0.0 };
    let p2 = Point { x: 3.0, y: 4.0 };
    let dist = distance(&p1, &p2);
    let mid = midpoint(&p1, &p2);
    println!("Distance: {}, Midpoint: ({}, {})", dist, mid.x, mid.y);

    let role = Role::Admin(Admin { id: 2, level: 5 });
    match role {
        Role::Guest => println!("Guest user"),
        Role::Member => println!("Regular member"),
        Role::Admin(admin) => println!("Admin id: {}, level: {}", admin.id, admin.level),
    }

    let numbers = vec![1, 2, 3, 4, 5];

    let doubled: Vec<i32> = numbers.iter().map(|x| x * 2).collect();
    println!("Doubled: {:?}", doubled);

    let multiply = |a: i32, b: i32| -> i32 { a * b };
    let product = multiply(6, 7);
    println!("6 * 7 = {}", product);

    let mut counter = 0;
    let mut increment = || {
        counter += 1;
        counter
    };
    println!("Count: {}", increment());
    println!("Count: {}", increment());
}
