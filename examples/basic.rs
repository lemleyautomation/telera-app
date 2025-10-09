//#![windows_subsystem = "windows"]

use telera_app::*;
use strum::EnumString;

#[derive(EnumString, Debug, Default, Clone, PartialEq)]
#[strum(crate = "self::strum")] 
enum BasicEvents {
    #[default]
    None,
    SquirrelClicked,
    LoremClicked,
    FileButtonClicked,
}

impl EventHandler for BasicEvents{
    type UserApplication = BasicApp;
    fn dispatch(&self, app: &mut Self::UserApplication, context: Option<EventContext>, api: &mut API) {
        match self {
            BasicEvents::FileButtonClicked => file_button_clicked_handler(app, context, api),
            BasicEvents::LoremClicked => lorem_clicked_handler(app, context, api),
            BasicEvents::SquirrelClicked => squirrel_clicked_handler(app, context, api),
            BasicEvents::None => {}
        }
    }
}

fn squirrel_clicked_handler(app: &mut BasicApp, _context: Option<EventContext>, _api: &mut API){
    app.selected_document = 0;
}

fn lorem_clicked_handler(app: &mut BasicApp, _context: Option<EventContext>, _api: &mut API){
    app.selected_document = 1;
}

fn file_button_clicked_handler(app: &mut BasicApp, _context: Option<EventContext>, _api: &mut API){
    app.file_menu_open = !app.file_menu_open;
}

pub struct Document {
    pub title: String,
    pub contents: String,
}

struct BasicApp {
    documents: Vec<Document>,
    selected_document: usize,
    file_menu_open: bool,
    search_bar: String,
}

impl App for BasicApp {
    fn initialize(&mut self, core: &mut API) {
        let new_window =
            winit::window::Window::default_attributes().with_inner_size(LogicalSize::new(800, 600));
        core.create_viewport("Main", "Main", new_window);
    }
}

impl ParserDataAccess<BasicEvents> for BasicApp {
    fn get_bool(&self, name: &str, list: &Option<ListData>) -> Option<bool> {
        match list {
            None => {
                if name == "file-menu-opened" {
                    return Some(self.file_menu_open);
                }
                return None;
            }
            Some(list) => {
                if list.src == "Documents" {
                    if name == "selected" {
                        if self.selected_document == list.index as usize {
                            return Some(true);
                        } else {
                            return Some(false);
                        }
                    }
                }
                None
            }
        }
    }
    fn get_text<'render_pass, 'application>(
        &'application self,
        name: &str,
        list: &Option<ListData>,
    ) -> Option<&'render_pass String>
    where
        'application: 'render_pass,
    {
        //println!("{:?}, {:?}", name, list);
        match list {
            None => {
                if name == "title" {
                    return Some(&self.documents.get(self.selected_document).unwrap().title);
                }
                else if name == "contents" {
                    return Some(&self.documents.get(self.selected_document).unwrap().contents);
                }
                else if name == "search_bar" {
                    return Some(&self.search_bar)
                }
                None
            }
            Some(list) => {
                if list.src == "Documents" {
                    if name == "title" {
                        //println!("asking for list element {:?} title", list.index);
                        return Some(&self.documents.get(list.index as usize).unwrap().title);
                    }
                    if name == "contents" {
                        return Some(&self.documents.get(list.index as usize).unwrap().contents);
                    }
                }
                None
            }
        }
    }
    fn get_list_length(&self, name: &str, _list: &Option<ListData>) -> Option<i32> {
        if name == "Documents" {
            return Some(self.documents.len() as i32);
        }
        None
    }
    fn get_event<'render_pass, 'application>(
        &'application self,
        name: &str,
        list: &Option<ListData>,
    ) -> Option<BasicEvents>
    where
        'application: 'render_pass,
    {
        match list {
            None => return None,
            Some(list) => {
                if name == "Clicked" && list.src == "Documents" {
                    match list.index {
                        0 => return Some(BasicEvents::SquirrelClicked),
                        1 => return Some(BasicEvents::LoremClicked),
                        _ => return None,
                    }
                }
                return None;
            }
        }
    }
}

fn main() {
    let mut documents = Vec::<Document>::new();

    documents.push(Document{
        title:"Squirrels".to_string(), 
        contents: "The Secret Life of Squirrels: Nature's Clever Acrobats\n\"Squirrels are often overlooked creatures, dismissed as mere park inhabitants or backyard nuisances. Yet, beneath their fluffy tails and twitching noses lies an intricate world of cunning, agility, and survival tactics that are nothing short of fascinating. As one of the most common mammals in North America, squirrels have adapted to a wide range of environments from bustling urban centers to tranquil forests and have developed a variety of unique behaviors that continue to intrigue scientists and nature enthusiasts alike.\n\"\n\"Master Tree Climbers\n\"At the heart of a squirrel's skill set is its impressive ability to navigate trees with ease. Whether they're darting from branch to branch or leaping across wide gaps, squirrels possess an innate talent for acrobatics. Their powerful hind legs, which are longer than their front legs, give them remarkable jumping power. With a tail that acts as a counterbalance, squirrels can leap distances of up to ten times the length of their body, making them some of the best aerial acrobats in the animal kingdom.\n\"But it's not just their agility that makes them exceptional climbers. Squirrels' sharp, curved claws allow them to grip tree bark with precision, while the soft pads on their feet provide traction on slippery surfaces. Their ability to run at high speeds and scale vertical trunks with ease is a testament to the evolutionary adaptations that have made them so successful in their arboreal habitats.\n\"\n\"Food Hoarders Extraordinaire\n\"Squirrels are often seen frantically gathering nuts, seeds, and even fungi in preparation for winter. While this behavior may seem like instinctual hoarding, it is actually a survival strategy that has been honed over millions of years. Known as \"scatter hoarding,\" squirrels store their food in a variety of hidden locations, often burying it deep in the soil or stashing it in hollowed-out tree trunks.\nInterestingly, squirrels have an incredible memory for the locations of their caches. Research has shown that they can remember thousands of hiding spots, often returning to them months later when food is scarce. However, they don't always recover every stash some forgotten caches eventually sprout into new trees, contributing to forest regeneration. This unintentional role as forest gardeners highlights the ecological importance of squirrels in their ecosystems.\n\nThe Great Squirrel Debate: Urban vs. Wild\nWhile squirrels are most commonly associated with rural or wooded areas, their adaptability has allowed them to thrive in urban environments as well. In cities, squirrels have become adept at finding food sources in places like parks, streets, and even garbage cans. However, their urban counterparts face unique challenges, including traffic, predators, and the lack of natural shelters. Despite these obstacles, squirrels in urban areas are often observed using human infrastructure such as buildings, bridges, and power lines as highways for their acrobatic escapades.\nThere is, however, a growing concern regarding the impact of urban life on squirrel populations. Pollution, deforestation, and the loss of natural habitats are making it more difficult for squirrels to find adequate food and shelter. As a result, conservationists are focusing on creating squirrel-friendly spaces within cities, with the goal of ensuring these resourceful creatures continue to thrive in both rural and urban landscapes.\n\nA Symbol of Resilience\nIn many cultures, squirrels are symbols of resourcefulness, adaptability, and preparation. Their ability to thrive in a variety of environments while navigating challenges with agility and grace serves as a reminder of the resilience inherent in nature. Whether you encounter them in a quiet forest, a city park, or your own backyard, squirrels are creatures that never fail to amaze with their endless energy and ingenuity.\nIn the end, squirrels may be small, but they are mighty in their ability to survive and thrive in a world that is constantly changing. So next time you spot one hopping across a branch or darting across your lawn, take a moment to appreciate the remarkable acrobat at work a true marvel of the natural world.\n".to_string()
    });

    documents.push(Document{
        title:"Lorem Ipsum".to_string(), 
        contents: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.".to_string()
    });

    let app = BasicApp {
        documents,
        selected_document: 1,
        file_menu_open: false,
        search_bar: "hello".to_string()
    };

    run::<BasicEvents, BasicApp>(app);
}
