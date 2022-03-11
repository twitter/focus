use crate::app::App;
use crate::coordinate::CoordinateSet;
use crate::model::layering::Layer;
use crate::model::layering::LayerSets;
use crate::model::repo::Repo;
use crate::operation::util::perform;
use crate::util::backed_up_file::BackedUpFile;

use std::convert::TryFrom;

use std::path::Path;

use std::sync::Arc;

use anyhow::{bail, Context, Result};

pub fn run(sparse_repo: &Path, app: Arc<App>) -> Result<()> {
    let repo = Repo::open(sparse_repo, app.clone()).context("Failed to open the repo")?;
    let sparse_profile_path = repo.git_dir().join("info").join("sparse-checkout");
    if !sparse_profile_path.is_file() {
        bail!("This does not appear to be a focused repo -- it is missing a sparse checkout file");
    }
    super::ensure_clean::run(sparse_repo, app.clone())
        .context("Failed trying to determine whether working trees were clean")?;

    let backed_up_sparse_profile = BackedUpFile::new(&sparse_profile_path)?;

    // Figure out all of the coordinates we will be resolving
    let coordinates = perform("Enumerating coordinates", || {
        let mut coordinates = Vec::<String>::new();
        let mut merge_coordinates_from_layer = |layer: &Layer| {
            let coordinates_in_layer: Vec<String> = layer
                .coordinates()
                .iter()
                .map(|coord| coord.to_owned())
                .collect::<_>();
            coordinates.extend(coordinates_in_layer);
        };

        // Add mandatory layers
        let sets = LayerSets::new(sparse_repo);
        let layer_set = sets
            .computed_layers()
            .context("Failed resolving applied layers")?;
        for layer in layer_set.layers() {
            merge_coordinates_from_layer(layer);
        }

        Ok(coordinates)
    })?;
    // Add coordinates count to TI custom map.
    app.tool_insights_client().get_context().add_to_custom_map(
        "coordinates_and_layers_count",
        coordinates.len().to_string(),
    );

    let coordinate_set =
        CoordinateSet::try_from(coordinates.as_ref()).context("constructing coordinate set")?;

    perform("Computing the new sparse profile", || {
        repo.sync(&coordinate_set, app).context("Sync failed")
    })?;

    perform("Updating the sync point", || {
        repo.working_tree().unwrap().write_sync_point_ref()
    })?;

    // The profile was successfully applied, so do not restore the backup.
    backed_up_sparse_profile.set_restore(false);

    Ok(())
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use anyhow::Result;

    use crate::{
        app,
        operation::{
            self,
            testing::integration::{RepoDisposition, RepoPairFixture},
        },
        testing::init_logging,
    };

    #[test]
    fn sync_upstream_changes() -> Result<()> {
        init_logging();

        let fixture = RepoPairFixture::new()?;

        fixture.perform_clone()?;

        // Commit new files affecting the build graph to the dense repo
        let build_bazel_content = r#"filegroup(
    name = "excerpts",
    srcs = [
        "catz.txt",
    ],
    visibility = [
        "//visibility:public",
    ],
)"#;
        fixture.dense_repo.commit(
            Path::new("x/BUILD.bazel"),
            build_bazel_content.as_bytes(),
            "Add excerpts",
        )?;
        let catz_txt_content = r#"The Naming of Cats is a difficult matter,
It isn't just one of your holiday games
        )"#;
        fixture.dense_repo.commit(
            Path::new("x/catz.txt"),
            catz_txt_content.as_bytes(),
            "Add excerpts",
        )?;

        // Fetch in the sparse repo from the dense repo
        fixture.perform_pull(RepoDisposition::Sparse, "origin", "master")?;

        // Make sure that the graph is seen as having changed
        assert_eq!(
            operation::detect_build_graph_changes::run(
                &fixture.sparse_repo_path,
                vec![],
                fixture.app.clone(),
            )?,
            app::ExitCode(1)
        );

        // Sync in the sparse repo
        operation::sync::run(&fixture.sparse_repo_path, fixture.app.clone())?;

        let x_dir = fixture.sparse_repo_path.join("x");
        assert!(!x_dir.is_dir());

        // Add as an ad-hoc coordinate
        operation::adhoc::push(
            fixture.sparse_repo_path.clone(),
            vec![String::from("bazel://x/...")],
        )?;

        // Sync
        operation::sync::run(&fixture.sparse_repo_path, fixture.app.clone())?;

        assert!(x_dir.is_dir());

        Ok(())
    }

    #[test]
    fn sync_layer_manipulation() -> Result<()> {
        init_logging();

        let fixture = RepoPairFixture::new()?;
        fixture.perform_clone()?;

        let path = fixture.sparse_repo_path.clone();
        let library_a_dir = path.join("library_a");
        let project_a_dir = path.join("project_a");
        let library_b_dir = path.join("library_b");
        let project_b_dir = path.join("project_b");

        assert!(!library_b_dir.is_dir());
        assert!(!project_b_dir.is_dir());
        operation::layer::push(&path, vec![String::from("team_zissou/project_b")])?;
        operation::sync::run(&path, fixture.app.clone())?;
        assert!(library_b_dir.is_dir());
        assert!(project_b_dir.is_dir());

        assert!(!library_a_dir.is_dir());
        assert!(!project_a_dir.is_dir());
        operation::layer::push(&path, vec![String::from("team_banzai/project_a")])?;
        operation::sync::run(&path, fixture.app.clone())?;
        assert!(library_a_dir.is_dir());
        assert!(project_a_dir.is_dir());

        operation::layer::pop(&path, 1)?;
        operation::sync::run(&path, fixture.app.clone())?;
        assert!(!library_a_dir.is_dir());
        assert!(!project_a_dir.is_dir());

        operation::layer::pop(&path, 1)?;
        operation::sync::run(&path, fixture.app.clone())?;
        assert!(!library_b_dir.is_dir());
        assert!(!project_b_dir.is_dir());

        Ok(())
    }

    #[test]
    fn sync_adhoc_manipulation() -> Result<()> {
        init_logging();

        let fixture = RepoPairFixture::new()?;
        fixture.perform_clone()?;

        let path = fixture.sparse_repo_path.clone();
        let library_b_dir = path.join("library_b");

        operation::adhoc::push(
            fixture.sparse_repo_path.clone(),
            vec![String::from("bazel://library_b/...")],
        )?;
        operation::sync::run(&path, fixture.app.clone())?;
        assert!(library_b_dir.is_dir());

        operation::adhoc::pop(fixture.sparse_repo_path.clone(), 1)?;
        operation::sync::run(&path, fixture.app.clone())?;
        assert!(!library_b_dir.is_dir());

        Ok(())
    }
}
