
use telera_app::process_layout;

fn main(){
    if let Ok(file) = std::fs::read_to_string("src/layouts/main.md") 
    && let Ok((_page_name, page_layout, _reusables)) = process_layout::<()>(file) {
        //println!("{:?}", layout_commands.unwrap().1.len());
        //pvec(&page_layout);
    }
}

fn pvec<T: std::fmt::Debug>(vec: &Vec<T>){
    println!("*******************************************************************");
    vec.iter().for_each(|element| println!("{:?}", element));
    println!("*******************************************************************");
}