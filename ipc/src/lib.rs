use std::io;

use libc::{c_int, ftok, semctl, semget, semop, IPC_CREAT, IPC_EXCL, IPC_PRIVATE, O_RDWR};
// use libc::{SETVAL, SETALL}; Can't find them for some reason
const SETVAL: c_int = 8;
const SETALL: c_int = 9;

use memmap::{MmapMut, MmapOptions};
use nix::unistd::{fork, ForkResult, Pid};

struct Semaphore<T> {
    users: usize,
    id: i32,
    data: T,
}

#[must_use = "if unused the Semaphore will immediately unlock"]
struct SemaphoreGuard<'a, T> {
    lock: &'a Semaphore<T>,
}

impl<T> Semaphore<T> {
    pub fn new(users: usize, data: T) -> Self {
        const FAILED_TO_OPEN_SEM_SET: i32 = -1;
        let path: *const str = std::env::current_exe().unwrap().to_str().unwrap();

        unsafe {
            let key = ftok(path.cast(), 1);
            let res = semget(key, users as i32, IPC_PRIVATE); // try to get semaphore from existing set, this should fail
            let id = if res == FAILED_TO_OPEN_SEM_SET {
                semget(key, users as i32, IPC_CREAT | IPC_EXCL | O_RDWR)
            // ToDo: Check error of semget
            } else {
                panic!("Semaphore at key {} already exists!", key)
            };
            if semctl(dbg!(id), 0, SETVAL, 1) == -1 {
                panic!("Failed to clear Semaphore");
            }
            Semaphore { users, id, data }
        }
    }

    pub fn from_id() -> Self {
        //semget
        unimplemented!()
    }

    pub fn lock<'a>(&'a self) -> SemaphoreGuard<'a, T> {
        //semop()
        unimplemented!()
    }
}
