pub struct FormatsRoot {
    s: String,
}

pub mod proto {
    tonic::include_proto!("treesnap");
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
