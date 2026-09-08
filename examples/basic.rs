#![cfg_attr(rustfmt, rustfmt_skip)]

use telera_app::*;
use telera_layout::{ElementConfiguration, TextConfig};
use winit::event::KeyEvent;

#[derive(Default)]
pub struct Document {
    pub title: String,
    pub contents: String,
}

#[derive(Default)]
struct BasicApp {
    documents: Vec<Document>,
    selected_document: usize,
    file_menu_open: bool,
    search_bar: String,
}

impl LayoutRunnerReflection for BasicApp {}
impl LayoutReflector for BasicApp {}

fn process_keyboard_input(feild: &String, input: KeyEvent) -> String {
    if let Some(input) = input.text {
        println!("{:?}", input);
        format!("{:?}{:?}", feild, input)
    }
    else {
        "".to_string()
    }
}

impl App for BasicApp {
    fn initialize(&mut self) -> Startup {
        Startup {
            window_attributes: Window::default_attributes().with_inner_size(LogicalSize::new(800, 600)),
            window_name: "Main".to_string(),
            page: None,
            watch_path: RunType::None
        }
    }

    #[layout_fn]
    fn layout(&mut  self, page: &str, api: &mut API) {
        for input in api.key_events().to_vec() {
            process_keyboard_input(&self.search_bar, input);
        }

        if page == "Main" {
            let main = ElementConfiguration::default()
                .grow()
                .color([43, 41, 51, 255].into())
                .vertical()
                .padding_all(16)
                .child_gap(16)
                .end();
            let header = ElementConfiguration::default()
                .color([90, 90, 90, 255].into())
                .radius_all(8.0)
                .width_grow()
                .height_fixed(60.0)
                .padding_top(8)
                .padding_bottom(8)
                .padding_left(16)
                .padding_right(16)
                .child_gap(16)
                .align_children_y_center()
                .end();
            let lower_content = ElementConfiguration::default()
                .child_gap(16)
                .grow()
                .end();
            let side_bar = ElementConfiguration::default()
                .color([90, 90, 90, 255].into())
                .vertical()
                .padding_all(16)
                .child_gap(8)
                .width_fixed(250.0)
                .height_grow()
                .radius_all(8.0)
                .end();
            let mut main_content = ElementConfiguration::default()
                .color([90,90,90,255].into())
                .vertical()
                .child_gap(16)
                .padding_all(16)
                .grow()
                .radius_all(8.0)
                .end();
            let side_bar_button = ElementConfiguration::default()
                .width_grow()
                .padding_all(16)
                .end();
            let selected_side_bar_button = ElementConfiguration::default()
                .width_grow()
                .padding_all(16)
                .color([120,120,120,255].into())
                .radius_all(8.0)
                .end();
            let clicked_side_bar_button = ElementConfiguration::default()
                .width_grow()
                .padding_all(16)
                .color([120,120,120,255].into())
                .border_all(2)
                .border_color([255,255,255,255].into())
                .radius_all(8.0)
                .end();
            let hovered_side_bar_button = ElementConfiguration::default()
                .width_grow()
                .padding_all(16)
                .color([120,120,120,255].into())
                .radius_all(8.0)
                .end();
            let file_button = ElementConfiguration::default()
                .padding_top(8)
                .padding_bottom(8)
                .padding_left(16)
                .padding_right(16)
                .color([140, 140, 140, 255].into())
                .radius_all(5.0)
                .end();
            let search_bar = ElementConfiguration::default()
                .padding_top(8)
                .padding_bottom(8)
                .padding_left(15)
                .padding_right(16)
                .color([255,255,255,255].into())
                .radius_all(5.0)
                .height_grow()
                .width_fixed(200.)
                .end();
            let hovered_file_button = ElementConfiguration::default()
                .padding_top(8)
                .padding_bottom(8)
                .padding_left(16)
                .padding_right(16)
                .color([140, 140, 140, 255].into())
                .radius_all(5.0)
                .border_all(2)
                .border_color([255, 255, 255, 255].into())
                .end();
            let context_menu = ElementConfiguration::default()
                .padding_bottom(8)
                .padding_right(8)
                .floating()
                .floating_attach_parent_bottom_left()
                .end();
            let context_pane = ElementConfiguration::default()
                .vertical()
                .width_fixed(200.0)
                .color([40,40,40,255].into())
                .radius_all(8.0)
                .end();
            let context_menu_item = ElementConfiguration::default()
                .padding_all(16)
                .width_grow()
                .end();
            let hovered_context_menu_item = ElementConfiguration::default()
                .padding_all(16)
                .color([120,120,120,255].into())
                .width_grow()
                .end();
            let text_config = TextConfig::new()
                .font_id(0)
                .font_color([0, 0, 0, 255].into())
                .font_size(12)
                .line_height(14)
                .end();
            let main_text_config = TextConfig::new()
                .font_id(0)
                .font_color([0,0,0,255].into())
                .font_size(24)
                .line_height(28)
                .end();

            e!(
                main,
                e!(
                    header,
                    e!(
                        if api.l.hovered() {
                            if api.left_mouse_clicked() {
                                self.file_menu_open = !self.file_menu_open;
                            }
                            hovered_file_button
                        } else {
                            file_button
                        },
                        t!(text_config, "File"),
                        if self.file_menu_open {
                            e!(
                                context_menu,
                                e!(
                                    context_pane,
                                    e!(
                                        if api.l.hovered() {
                                            hovered_context_menu_item
                                        }
                                        else {
                                            context_menu_item
                                        },
                                        t!(text_config, "New")
                                    ),
                                    e!(
                                        if api.l.hovered() {
                                            hovered_context_menu_item
                                        }
                                        else {
                                            context_menu_item
                                        },
                                        t!(text_config, "Open")
                                    ),
                                    e!(
                                        if api.l.hovered() {
                                            hovered_context_menu_item
                                        }
                                        else {
                                            context_menu_item
                                        },
                                        t!(text_config, "Save")
                                    ),
                                )
                            );
                        }
                    ),
                    e!(search_bar,t!(text_config,&self.search_bar)),
                    e!(ElementConfiguration::default().width_grow().end()),
                    e!(
                        if api.l.hovered() {
                            hovered_file_button
                        } else {
                            file_button
                        },
                        t!(text_config, "Media")
                    ),
                    e!(
                        if api.l.hovered() {
                            hovered_file_button
                        } else {
                            file_button
                        },
                        t!(text_config, "Support")
                    ),
                ),
                e!(
                    lower_content,
                    e!(
                        side_bar,
                        for i in 0..self.documents.len() {
                            api.l.open_element();
                            if api.l.hovered() && api.left_mouse_clicked() {
                                api.l.configure_element(&clicked_side_bar_button);
                                self.selected_document = i;
                            }
                            else if api.l.hovered() {
                                api.l.configure_element(&hovered_side_bar_button);
                            }
                            else if self.documents[self.selected_document].title == self.documents[i].title {
                                api.l.configure_element(&selected_side_bar_button);
                            }
                            else {
                                api.l.configure_element(&side_bar_button);
                            }
                            api.l.add_text_element(&self.documents[i].title, &text_config, true);
                            api.l.close_element();
                        }
                    ),
                    e!(
                        {
                            let offset = api.l.get_scroll_offset();
                            main_content.scroll(true, false).scroll_child_offset(offset.x, offset.y).end()
                        },
                        t!(main_text_config, &self.documents[self.selected_document].title),
                        t!(main_text_config, &self.documents[self.selected_document].contents)
                    )
                )
            );
        }
    }
}

fn main() {
    let documents = vec![
        Document{
            title:"Squirrels".to_string(),
            contents: "The Secret Life of Squirrels: Nature's Clever Acrobats\n\"Squirrels are often overlooked creatures, dismissed as mere park inhabitants or backyard nuisances. Yet, beneath their fluffy tails and twitching noses lies an intricate world of cunning, agility, and survival tactics that are nothing short of fascinating. As one of the most common mammals in North America, squirrels have adapted to a wide range of environments from bustling urban centers to tranquil forests and have developed a variety of unique behaviors that continue to intrigue scientists and nature enthusiasts alike.\n\"\n\"Master Tree Climbers\n\"At the heart of a squirrel's skill set is its impressive ability to navigate trees with ease. Whether they're darting from branch to branch or leaping across wide gaps, squirrels possess an innate talent for acrobatics. Their powerful hind legs, which are longer than their front legs, give them remarkable jumping power. With a tail that acts as a counterbalance, squirrels can leap distances of up to ten times the length of their body, making them some of the best aerial acrobats in the animal kingdom.\n\"But it's not just their agility that makes them exceptional climbers. Squirrels' sharp, curved claws allow them to grip tree bark with precision, while the soft pads on their feet provide traction on slippery surfaces. Their ability to run at high speeds and scale vertical trunks with ease is a testament to the evolutionary adaptations that have made them so successful in their arboreal habitats.\n\"\n\"Food Hoarders Extraordinaire\n\"Squirrels are often seen frantically gathering nuts, seeds, and even fungi in preparation for winter. While this behavior may seem like instinctual hoarding, it is actually a survival strategy that has been honed over millions of years. Known as \"scatter hoarding,\" squirrels store their food in a variety of hidden locations, often burying it deep in the soil or stashing it in hollowed-out tree trunks.\nInterestingly, squirrels have an incredible memory for the locations of their caches. Research has shown that they can remember thousands of hiding spots, often returning to them months later when food is scarce. However, they don't always recover every stash some forgotten caches eventually sprout into new trees, contributing to forest regeneration. This unintentional role as forest gardeners highlights the ecological importance of squirrels in their ecosystems.\n\nThe Great Squirrel Debate: Urban vs. Wild\nWhile squirrels are most commonly associated with rural or wooded areas, their adaptability has allowed them to thrive in urban environments as well. In cities, squirrels have become adept at finding food sources in places like parks, streets, and even garbage cans. However, their urban counterparts face unique challenges, including traffic, predators, and the lack of natural shelters. Despite these obstacles, squirrels in urban areas are often observed using human infrastructure such as buildings, bridges, and power lines as highways for their acrobatic escapades.\nThere is, however, a growing concern regarding the impact of urban life on squirrel populations. Pollution, deforestation, and the loss of natural habitats are making it more difficult for squirrels to find adequate food and shelter. As a result, conservationists are focusing on creating squirrel-friendly spaces within cities, with the goal of ensuring these resourceful creatures continue to thrive in both rural and urban landscapes.\n\nA Symbol of Resilience\nIn many cultures, squirrels are symbols of resourcefulness, adaptability, and preparation. Their ability to thrive in a variety of environments while navigating challenges with agility and grace serves as a reminder of the resilience inherent in nature. Whether you encounter them in a quiet forest, a city park, or your own backyard, squirrels are creatures that never fail to amaze with their endless energy and ingenuity.\nIn the end, squirrels may be small, but they are mighty in their ability to survive and thrive in a world that is constantly changing. So next time you spot one hopping across a branch or darting across your lawn, take a moment to appreciate the remarkable acrobat at work a true marvel of the natural world.\n".to_string()
        },
        Document{
            title:"Lorem Ipsum".to_string(),
            contents: "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.".to_string()
        }
    ];

    let app = BasicApp {
        documents,
        selected_document: 1,
        file_menu_open: false,
        search_bar: "hello".to_string(),
    };

    run::<BasicApp>(app);
}
