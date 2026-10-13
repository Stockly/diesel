extern crate diesel;

use diesel::*;

table! {
    users {
        id -> Integer,
        name -> Text,
    }
}

fn main() {
    let mut conn = PgConnection::establish("").unwrap();
    // A select set before group_by is only checked against the GROUP BY clause
    // once the query is loaded
    let _ = users::table
        .select((users::id, users::name))
        .group_by(users::name)
        .load::<(i32, String)>(&mut conn);
    //~^ ERROR: type mismatch resolving `<name as IsContainedInGroupBy<id>>::Output == Yes`
}
