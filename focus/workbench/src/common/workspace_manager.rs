pub struct WorkspaceManager {
    workspaces: Mutex<HashMap<Uuid, Arc<Mutex<Workspace>>>>,
}

impl WorkspaceManager {
}

impl Default for WorkspaceManager {

}

pub struct Workspace {
    uuid: Uuid,

    parent: Option<Arc<Workspace>>,

    path: PathBuf,
}
