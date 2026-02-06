use omni_shared::project::Project;
use omni_engine::nodes::AudioNode;
use omni_engine::nodes::GainNode;
use omni_engine::plugin_node::PluginNode;
use std::fs::File;
use std::io::Write;

pub fn load_project_file(path: &str, sample_rate: f64) -> Result<(Project, Vec<Box<dyn AudioNode>>), anyhow::Error> {
    let content = std::fs::read_to_string(path)?;
    let project: Project = serde_json::from_str(&content)?;
    
    let mut nodes: Vec<Box<dyn AudioNode>> = Vec::new();
    
    eprintln!("[ProjectIO] Loading Plugins for project: {}", project.name);

    for track in &project.tracks {
         if !track.plugin_path.is_empty() {
             match PluginNode::new(&track.plugin_path, sample_rate) {
                 Ok(n) => nodes.push(Box::new(n)),
                 Err(e) => {
                     eprintln!("[ProjectIO] Plugin Load Error: {}. Using GainNode.", e);
                     nodes.push(Box::new(GainNode::new(1.0)));
                 }
             }
         } else {
             nodes.push(Box::new(GainNode::new(1.0)));
         }
    }
    
    Ok((project, nodes))
}

pub fn save_project_file(project: &Project, path: &str) -> Result<(), anyhow::Error> {
    let json = serde_json::to_string_pretty(project)?;
    let mut file = File::create(path)?;
    file.write_all(json.as_bytes())?;
    Ok(())
}
