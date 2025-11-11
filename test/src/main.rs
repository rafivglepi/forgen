const PI: f64 = 3.1415;
const MAX_USERS: u32 = 100;

type UserId = u64;
type Score = f32;

pub struct User {
    pub id: UserId,
    pub name: String,
    score: Score,
}

struct Admin {
    pub id: UserId,
    level: u8,
}

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

    let greeting = alice.greet();
    println!("{}", greeting);

    let sum = add(5, 7);
    println!("5 + 7 = {}", sum);

    let role = Role::Admin(Admin { id: 2, level: 5 });
    match role {
        Role::Guest => println!("Guest user"),
        Role::Member => println!("Regular member"),
        Role::Admin(admin) => println!("Admin id: {}, level: {}", admin.id, admin.level),
    }
}
