mod lib;

fn main() {
    lib::bench_data_rate();
    println!("Servo:");
    lib::bench_data_rate_servo();
}
