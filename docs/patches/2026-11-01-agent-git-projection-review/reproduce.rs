#![allow(dead_code)]
use std::collections::HashMap;
mod dozer_panel_agent {
 pub mod tabs {
  #[derive(Debug,Clone,PartialEq,Eq)] pub struct WorkspaceGitInfo { pub branch:Option<String>,pub dirty:bool }
  pub struct SessionTab { pub tab_id:usize,pub cwd:String,pub workspace_override:Option<WorkspaceGitInfo> }
 }
 pub mod card {
  use super::tabs::{SessionTab,WorkspaceGitInfo};
pub fn apply_workspace_override(
    tabs: &mut [SessionTab],
    tab_id: usize,
    workspace: Option<WorkspaceGitInfo>,
) {
    if let Some(tab) = tabs.iter_mut().find(|t| t.tab_id == tab_id) {
        tab.workspace_override = workspace;
    }
}
 }
}
use dozer_panel_agent::tabs::*;
struct Agent {tabs:Vec<SessionTab>}
struct Workspace {agent:Agent}
fn loaded_workspace_mut(projects:&mut HashMap<i64,Workspace>,id:i64)->Option<&mut Workspace>{projects.get_mut(&id)}
enum Message {AgentGitProjection(i64,u64,Vec<usize>,Option<String>,bool)}
struct App {token:u64,projects:HashMap<i64,Workspace>}
impl App {
 fn git_token_current(&self,token:&u64)->bool{self.token==*token}
 fn handle(&mut self,message:Message){match message{
            Message::AgentGitProjection(project_id, token, tab_ids, branch, dirty) => {
                if self.git_token_current(&token)
                    && let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id)
                {
                    let info = dozer_panel_agent::tabs::WorkspaceGitInfo { branch, dirty };
                    for tab_id in tab_ids {
                        dozer_panel_agent::card::apply_workspace_override(
                            &mut ws.agent.tabs,
                            tab_id,
                            Some(info.clone()),
                        );
                    }
                }
            }
 }}
}
fn app()->App{App{token:7,projects:HashMap::from([(1,Workspace{agent:Agent{tabs:vec![SessionTab{tab_id:1,cwd:"/repo".into(),workspace_override:None}]}})])}}
fn result(branch:&str,dirty:bool)->Message{Message::AgentGitProjection(1,7,vec![1],Some(branch.into()),dirty)}
#[test]
fn old_projection_must_not_overwrite_a_newer_snapshot_projection(){
 let mut app=app();
 // Same token, two snapshots. Identity resolution for the older one finishes last.
 app.handle(result("new-head",false));
 app.handle(result("old-head",true));
 let card=app.projects[&1].agent.tabs[0].workspace_override.as_ref().unwrap();
 assert_eq!(card.branch.as_deref(),Some("new-head"),"late old projection replaced newer snapshot");
}
#[test]
fn old_projection_must_not_overwrite_a_tab_after_cwd_changes(){
 let mut app=app();
 let tab=&mut app.projects.get_mut(&1).unwrap().agent.tabs[0];
 tab.cwd="/repo/.worktrees/other".into();
 tab.workspace_override=Some(WorkspaceGitInfo{branch:Some("inner-worktree".into()),dirty:false});
 app.handle(result("parent",true));
 assert_eq!(app.projects[&1].agent.tabs[0].workspace_override.as_ref().unwrap().branch.as_deref(),Some("inner-worktree"),"projection used stale cwd membership");
}
