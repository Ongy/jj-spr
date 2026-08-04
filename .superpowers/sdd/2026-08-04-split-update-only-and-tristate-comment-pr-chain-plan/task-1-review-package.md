# Task 1 Review Package

Commit `kpwwwzov` (`f8b810f90ca747`)

```diff
diff --git a/spr/src/commands/push.rs b/spr/src/commands/push.rs
index 598109b659..04111ca355 100644
--- a/spr/src/commands/push.rs
+++ b/spr/src/commands/push.rs
@@ -29,6 +29,9 @@
 
     #[clap(long, num_args = 0..=1, default_missing_value = "true")]
     pub draft: Option<bool>,
+
+    #[clap(long, num_args = 0..=1, default_missing_value = "true")]
+    pub update_only: Option<bool>,
 }

 #[cfg(test)]
@@ -68,6 +71,11 @@
         self.draft = Some(val);
         self
     }
+
+    pub fn with_update_only(mut self, val: bool) -> Self {
+        self.update_only = Some(val);
+        self
+    }
 }

 enum WorkEvent<'a> {
@@ -622,6 +630,13 @@
         .pull_requests(workset.iter().map(|ws| ws.revision.pull_request_number))
         .await?;
 
+    let update_only = opts.update_only.unwrap_or(config.push.update_only);
+    if update_only && pull_requests.iter().any(|pr| pr.is_none()) {
+        return Err(Error::new(
+            "Cannot create new pull requests because --update-only is enabled",
+        ));
+    }
+
     let trunk = {
         let branch = jj.git_repo.find_branch(
             format!("{}/{}", config.remote_name, config.master_ref).as_ref(),
diff --git a/spr/src/config/main.rs b/spr/src/config/main.rs
index 83cca07648..e7d7aacaa8 100644
--- a/spr/src/config/main.rs
+++ b/spr/src/config/main.rs
@@ -667,5 +667,17 @@
             let parsed = parsed_from_jj(&jj).expect("Shouldn't fail to parse config");
             assert!(!parsed.push.draft);
         }
+
+        #[test]
+        fn test_parse_push_update_only() {
+            let (_tmpdir, mut jj, _) = testing::setup::repo_with_origin();
+
+            jj.config_set("spr.push.updateOnly", "true", false)
+                .expect("Failed to set spr.push.updateOnly");
+
+            let parsed = parsed_from_jj(&jj).expect("Shouldn't fail to parse config");
+            assert!(parsed.push.update_only);
+        }
     }
 }
diff --git a/spr/src/config/push.rs b/spr/src/config/push.rs
index ca8465e694..da469690e8 100644
--- a/spr/src/config/push.rs
+++ b/spr/src/config/push.rs
@@ -8,12 +8,19 @@
     false
 }
 
+fn default_update_only() -> bool {
+    false
+}
+
 #[derive(Serialize, Deserialize, Clone, Debug)]
+#[serde(rename_all = "camelCase")]
 pub struct PushConfig {
     #[serde(default = "default_autofix")]
     pub autofix: bool,
     #[serde(default = "default_draft")]
     pub draft: bool,
+    #[serde(default = "default_update_only", alias = "update_only")]
+    pub update_only: bool,
 }
 
 impl Default for PushConfig {
@@ -21,6 +28,39 @@
         Self {
             autofix: false,
             draft: false,
+            update_only: false,
         }
     }
 }
```
