use std::time::Instant;

use telera_app::{*, event_handler_derive::EventHandler};
use strum::EnumString;


use telera_app::process_layout;

struct BasicApp{

}

#[derive(EnumString, Debug, Clone, PartialEq, EventHandler)]
#[handler_for(BasicApp)]
#[strum(crate = "self::strum")] 
enum BasicEvents {
}

fn main(){
    let mut binder = Binder::<BasicEvents>::new();
    
    if let Ok(file) = std::fs::read_to_string("src/layouts/main.md") {
        //let dt = Instant::now();
        if let Ok((page_name, page_layout, reusables)) = process_layout::<BasicEvents>(file) {
            //let dtt = Instant::now();
            pvec(&page_layout);
            binder.add_page(&page_name, page_layout);
            for re in reusables {
                binder.add_reusable(&re.0, re.1);
            }
            //println!("binding time: {:}", dtt.elapsed().as_micros());
        }
        //println!("parsing time: {:}", dt.elapsed().as_micros());
    }
}

fn pvec<T: std::fmt::Debug>(vec: &Vec<T>){
    println!("*******************************************************************");
    vec.iter().for_each(|element| println!("{:?}", element));
    println!("*******************************************************************");
}