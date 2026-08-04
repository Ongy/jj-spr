# Task 2 Review Package

Revision `wnkpkovk` (`b703f15fea1ddc`) stacked on `kpwwwzov`.

```diff
diff --git a/spr/src/commands/push.rs b/spr/src/commands/push.rs
index 04111ca355..b703f15fea 100644
--- a/spr/src/commands/push.rs
+++ b/spr/src/commands/push.rs
@@ -1,5 +1,6 @@
 use crate::{
     error::{Error, Result, ResultExt},
+    config::push::CommentMode,
     jj::RevSet,
     message::{MessageSection, build_github_body},
     utils::run_command,
@@ -32,6 +33,9 @@
 
     #[clap(long, num_args = 0..=1, default_missing_value = "true")]
     pub update_only: Option<bool>,
+
+    #[clap(long, value_enum)]
+    pub modify_comment: Option<CommentMode>,
 }
 
 #[cfg(test)]
@@ -76,6 +80,11 @@
         self.update_only = Some(val);
         self
     }
+
+    pub fn with_modify_comment(mut self, val: CommentMode) -> Self {
+        self.modify_comment = Some(val);
+        self
+    }
 }

@@ -754,6 +763,16 @@
     for ws in actions.iter() {
         ws.progress_bar
             .finish_with_message(ws.format_worklog(config));
     }
+
+    let modify_comment = opts
+        .modify_comment
+        .unwrap_or(config.push.modify_comment);

+    let should_update_comments = match modify_comment {
+        CommentMode::Off => false,
+        CommentMode::On => true,
+        CommentMode::Auto => actions.len() > 1,
+    };
+
+    if should_update_comments {
+        update_tree_overviews(&mut gh, config, actions, &setup).await?;
+    }
     setup.finish_and_clear();
```
