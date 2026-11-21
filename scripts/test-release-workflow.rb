#!/usr/bin/env ruby
# frozen_string_literal: true

# Checks lt's release pipeline (modelled on the ligus repo's
# scripts/test-release-workflow.rb):
#
#   - the binaries land on the tag's one DRAFT release and nothing is
#     published until every build succeeded and the draft holds every asset;
#   - promotion to Latest stays manual;
#   - ci.yml builds every target the release ships, through the same action,
#     and ci-ok needs every CI job;
#   - the formula follows publish, goes through tap-push.sh, and never prints
#     the tap token (the formula job is run here against stubs and a local tap);
#   - macOS signing uses lt's signing identifier and skips cleanly without
#     the secrets;
#   - cut-release.yml checks admin, ci-ok on the exact commit and the version
#     before it pushes the tag, then calls release.yml.
#
#   ruby scripts/test-release-workflow.rb

require "json"
require "minitest/autorun"
require "open3"
require "tmpdir"
require "fileutils"
require "yaml"

ROOT = File.expand_path("..", __dir__)
def load_yaml(path)
  YAML.safe_load(File.read(File.join(ROOT, path)))
end
RELEASE = load_yaml(".github/workflows/release.yml")
CI = load_yaml(".github/workflows/ci.yml")
CUT = load_yaml(".github/workflows/cut-release.yml")
BUILD_LT = load_yaml(".github/actions/build-lt/action.yml")
TARGETS = %w[aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-musl aarch64-unknown-linux-musl].freeze

def steps(workflow, job)
  workflow.fetch("jobs").fetch(job).fetch("steps")
end

def run_text(workflow, job)
  steps(workflow, job).map { |s| s["run"].to_s }.join("\n")
end

# YAML parses the bare key `on` as true.
def triggers(workflow)
  workflow["on"] || workflow[true]
end

def matrix_targets(workflow, job)
  workflow.fetch("jobs").fetch(job).dig("strategy", "matrix", "include").map { |e| [e["target"], e["runner"]] }.to_h
end

def step_named(workflow, job, name)
  s = steps(workflow, job).find { |st| st["name"] == name }
  raise "#{job} has no step #{name}" unless s

  s
end

# A bash new enough for the formula job (declare -A): bash 4+.
def modern_bash
  ["bash", "/opt/homebrew/bin/bash", "/usr/local/bin/bash"].find do |b|
    out, st = Open3.capture2e(b, "-c", "echo ${BASH_VERSINFO[0]}")
    st.success? && out.to_i >= 4
  rescue SystemCallError
    false
  end
end

class Targets < Minitest::Test
  def test_release_ships_the_four_targets
    release = matrix_targets(RELEASE, "build")
    assert_equal TARGETS.sort, release.keys.sort
    assert_equal "macos-latest", release["aarch64-apple-darwin"]
    assert_equal "macos-latest", release["x86_64-apple-darwin"]
    assert_equal "ubuntu-24.04", release["x86_64-unknown-linux-musl"]
    assert_equal "ubuntu-24.04-arm", release["aarch64-unknown-linux-musl"]
  end

  def test_ci_builds_every_release_target_on_the_same_runner
    assert_equal matrix_targets(RELEASE, "build"), matrix_targets(CI, "targets")
  end

  def test_both_build_through_the_shared_action
    rel = steps(RELEASE, "build").find { |s| s["uses"] == "./.github/actions/build-lt" }
    ci = steps(CI, "targets").find { |s| s["uses"] == "./.github/actions/build-lt" }
    refute_nil rel
    refute_nil ci
    assert_equal "release", rel.dig("with", "profile")
    assert_equal "debug", ci.dig("with", "profile")
    [rel, ci].each { |s| assert_equal "${{ matrix.target }}", s.dig("with", "target") }
    # Nobody builds lt beside the action, where it could drift.
    [run_text(RELEASE, "build"), run_text(CI, "targets")].each do |t|
      refute_match(/cargo build|rustup|musl-tools/, t)
    end
  end

  def test_shared_action_sets_up_the_target
    action_steps = BUILD_LT.dig("runs", "steps")
    text = action_steps.map { |s| s["run"].to_s }.join("\n")
    assert_includes text, 'rustup target add "$TARGET"'
    assert_match(/cargo build .*--locked --bin lt --target "\$TARGET"/, text)
    assert_includes text, "scripts/check-no-test-relay.sh"
    assert_includes text, "statically linked"
    musl = action_steps.find { |s| s["run"].to_s.include?("musl-tools") }
    assert_equal "contains(inputs.target, 'linux-musl')", musl["if"]
  end

  def test_asset_check_and_formula_use_the_same_targets
    script = File.read(File.join(ROOT, "scripts/check-release-assets.py"))
    listed = script[/^TARGETS = \((.*?)\)/m, 1].scan(/"([^"]+)"/).flatten
    assert_equal TARGETS.sort, listed.sort
    formula = run_text(RELEASE, "formula")[/for t in ([^;]+);/, 1].split
    assert_equal TARGETS.sort, formula.sort
  end
end

class CiOk < Minitest::Test
  def test_ci_ok_needs_every_other_job
    others = CI.fetch("jobs").keys - ["ci-ok"]
    assert_equal others.sort, Array(CI.dig("jobs", "ci-ok", "needs")).sort
    assert_equal "always()", CI.dig("jobs", "ci-ok", "if")
    gate = CI.dig("jobs", "ci-ok", "steps").find { |s| s["run"] == "exit 1" }
    %w[failure cancelled skipped].each { |r| assert_includes gate["if"], "'#{r}'" }
  end

  def test_ci_runs_on_main_pushes_and_prs
    assert_equal ["main"], triggers(CI)["push"]["branches"]
    assert triggers(CI).key?("pull_request")
  end

  def test_ci_runs_the_release_tooling_tests
    text = run_text(CI, "release-tooling")
    assert_includes text, "ruby scripts/test-release-workflow.rb"
    assert_includes text, "bash scripts/test-tap-push.sh"
    assert_includes text, "shellcheck scripts/*.sh"
  end
end

# The release stays a draft until every build succeeded: one job publishes,
# it waits on everything, and the formula follows it.
class DraftUntilComplete < Minitest::Test
  BUILD_JOBS = %w[release build].freeze

  def publish
    RELEASE.fetch("jobs").fetch("publish")
  end

  def test_publish_needs_every_build
    assert_equal BUILD_JOBS.sort, Array(publish["needs"]).sort
  end

  def test_publish_requires_each_build_to_succeed
    cond = publish["if"].to_s
    BUILD_JOBS.each do |j|
      assert_includes cond, "needs.#{j}.result == 'success'", "publish does not wait for #{j} to succeed"
    end
    assert_includes cond, "!cancelled()"
    refute_match(/always\(\)|failure\(\)/, cond)
  end

  def test_publish_checks_the_draft_then_publishes_a_prerelease_by_id
    st = publish["steps"]
    assets = st.index { |s| s["run"].to_s.include?("scripts/check-release-assets.py") }
    patch = st.index { |s| s["run"].to_s.include?("-X PATCH") }
    refute_nil assets, "publish does not check the draft's assets"
    refute_nil patch, "publish never publishes"
    assert_operator assets, :<, patch
    run = st[patch]["run"]
    assert_includes run, "releases/$RELEASE_ID"
    assert_includes run, "draft=false"
    assert_includes run, "prerelease=true"
    assert_includes run, "make_latest=false", "promotion to Latest stays manual"
    refute_includes run, "--latest"
    assert_operator run.index('"true $TAG"'), :<, run.index("-X PATCH"), "publish must check it holds the tag's draft"
  end

  def test_only_publish_publishes
    RELEASE["jobs"].each do |name, job|
      next if name == "publish"

      text = job.fetch("steps", []).map { |s| s["run"].to_s }.join("\n")
      refute_match(/draft=false|-X PATCH|release edit|--latest|gh release create/, text, "#{name} changes the release's state")
      job.fetch("steps", []).each do |s|
        refute s["uses"].to_s.start_with?("softprops/action-gh-release@"), "#{name} would publish the draft"
      end
    end
  end

  def test_release_job_checks_the_version_and_creates_a_draft
    text = run_text(RELEASE, "release")
    assert_operator text.index("scripts/check-release-config.sh"), :<, text.index("scripts/ensure-release.sh")
    script = File.read(File.join(ROOT, "scripts/ensure-release.sh"))
    assert_match(/-F draft=true/, script)
    assert_match(/-F prerelease=true/, script)
    refute_match(/gh release create/, script)
  end

  def test_builds_upload_only_into_the_draft
    st = steps(RELEASE, "build")
    up = st.find { |s| s["run"].to_s.include?("gh release upload") }
    assert_includes up["run"], '"$RELEASE_ID true"'
    assert_operator up["run"].index('"$RELEASE_ID true"'), :<, up["run"].index("gh release upload")
    assert_includes up["run"], '"lt-$TARGET" "lt-$TARGET.sha256"'
    stage = step_named(RELEASE, "build", "Stage asset")
    assert_includes stage["run"], 'install -m 0755 "$BIN" "lt-$TARGET"'
  end

  def test_build_checks_the_tag_before_building
    st = steps(RELEASE, "build")
    check = st.index { |s| s["run"].to_s.include?("check-release-config.sh") }
    build = st.index { |s| s["uses"] == "./.github/actions/build-lt" }
    assert_operator check, :<, build
  end

  def test_every_job_builds_the_tag_not_the_branch
    RELEASE["jobs"].each do |name, job|
      job.fetch("steps", []).select { |s| s["uses"].to_s.start_with?("actions/checkout@") }.each do |s|
        assert_includes s.dig("with", "ref").to_s, "refs/tags/", "#{name} checks out something other than the tag"
        assert_equal false, s.dig("with", "persist-credentials"), "#{name} keeps the checkout token"
      end
    end
  end

  def test_one_release_run_per_tag_never_cancelled
    c = RELEASE.fetch("concurrency")
    assert_includes c["group"], "inputs.tag || github.ref_name"
    assert_equal false, c["cancel-in-progress"]
  end

  def test_tag_push_fallback_and_call_triggers
    assert_equal ["v*"], triggers(RELEASE)["push"]["tags"]
    assert triggers(RELEASE)["workflow_call"]["inputs"]["tag"]["required"]
    assert triggers(RELEASE).key?("workflow_dispatch")
  end
end

class Formula < Minitest::Test
  def job
    RELEASE.fetch("jobs").fetch("formula")
  end

  def test_formula_runs_after_publish
    assert_equal ["publish"], Array(job["needs"])
    assert_includes job["if"].to_s, "needs.publish.result == 'success'"
    assert_includes job["if"].to_s, "!cancelled()"
  end

  def test_formula_goes_through_tap_push
    text = run_text(RELEASE, "formula")
    assert_includes text, "scripts/tap-push.sh tap"
    assert_includes text, "Formula/lt.rb"
    refute_match(/git push/, text, "only tap-push.sh pushes to the tap")
  end

  def test_formula_skips_cleanly_without_the_token
    text = run_text(RELEASE, "formula")
    assert_operator text.index('if [ -z "${TAP_TOKEN:-}" ]'), :<, text.index("git clone")
    assert_equal "${{ secrets.HOMEBREW_TAP_TOKEN }}", job["steps"].last.dig("env", "TAP_TOKEN")
  end

  def test_no_token_echo
    text = run_text(RELEASE, "formula")
    refute_match(/set -x|set -o xtrace|bash -x/, text)
    text.lines.grep(/TAP_TOKEN/).each do |l|
      refute_match(/echo|printf|cat /, l.sub(/echo "HOMEBREW_TAP_TOKEN not set[^"]*"/, ""), "prints the token: #{l}")
    end
    push = File.read(File.join(ROOT, "scripts/tap-push.sh"))
    refute_match(/set -x|remote get-url|remote -v/, push)
  end

  # Runs the formula step itself, with gh and the tap's git remote stubbed,
  # and checks the formula it pushes and that the token is never printed.
  def test_the_job_writes_a_working_formula_and_never_prints_the_token
    bash = modern_bash
    skip "needs bash 4+ (declare -A), as on the ubuntu runner" unless bash
    run = job["steps"].last["run"]
    token = "ghp_SECRET_TOKEN_must_not_print_1234"
    Dir.mktmpdir do |dir|
      env = { "GIT_CONFIG_GLOBAL" => "/dev/null", "GIT_CONFIG_NOSYSTEM" => "1" }
      tap = File.join(dir, "tap.git")
      seed = File.join(dir, "seed")
      sh = lambda do |*cmd|
        out, st = Open3.capture2e(env, *cmd)
        assert st.success?, "#{cmd.join(' ')}: #{out}"
      end
      sh.call("git", "init", "-q", "--bare", "-b", "main", tap)
      sh.call("git", "init", "-q", "-b", "main", seed)
      FileUtils.mkdir_p(File.join(seed, "Formula"))
      File.write(File.join(seed, "Formula/ligus.rb"), "class Ligus < Formula\nend\n")
      sh.call("git", "-C", seed, "add", "-A")
      sh.call("git", "-C", seed, "-c", "user.name=s", "-c", "user.email=s@example.com", "commit", "-q", "-m", "seed")
      sh.call("git", "-C", seed, "push", "-q", tap, "main")

      stubs = File.join(dir, "bin")
      FileUtils.mkdir_p(stubs)
      real_git = `sh -c 'command -v git'`.strip
      # gh release download <tag> --repo R --pattern P --output O --clobber
      File.write(File.join(stubs, "gh"), <<~SH)
        #!/bin/sh
        out=""; pat=""
        while [ $# -gt 0 ]; do
          case "$1" in --output) out=$2; shift ;; --pattern) pat=$2; shift ;; esac
          shift
        done
        printf 'binary %s\\n' "$pat" > "$out"
      SH
      # git clone <token url> tap -> clone the local tap instead.
      File.write(File.join(stubs, "git"), <<~SH)
        #!/bin/sh
        if [ "$1" = clone ]; then
          for a in "$@"; do last=$a; done
          exec #{real_git} clone -q "#{tap}" "$last"
        fi
        exec #{real_git} "$@"
      SH
      File.chmod(0o755, File.join(stubs, "gh"), File.join(stubs, "git"))
      work = File.join(dir, "work")
      FileUtils.mkdir_p(work)
      FileUtils.ln_s(File.join(ROOT, "scripts"), File.join(work, "scripts"))
      script = File.join(dir, "formula.sh")
      File.write(script, run)
      run_env = env.merge(
        "PATH" => "#{stubs}:#{ENV['PATH']}", "TAP_TOKEN" => token, "GH_TOKEN" => "x",
        "TAG" => "v0.1.1", "GITHUB_REPOSITORY" => "langstroth-ai/ligus-term", "RUNNER_TEMP" => dir
      )
      out, st = Open3.capture2e(run_env, bash, script, chdir: work)
      assert st.success?, out
      refute_includes out, token, "the formula job printed the tap token"
      assert_includes out, "Pushed 'lt 0.1.1' to the tap"

      formula = `git --git-dir=#{tap} show main:Formula/lt.rb`
      assert_match(/\Aclass Lt < Formula\n/, formula, "the formula must start flush, not YAML-indented")
      assert_includes formula, 'version "0.1.1"'
      TARGETS.each do |t|
        assert_includes formula, %(url "https://github.com/langstroth-ai/ligus-term/releases/download/v0.1.1/lt-#{t}")
        sha = `printf 'binary %s\\n' lt-#{t} | shasum -a 256`.split.first
        assert_includes formula, %(sha256 "#{sha}")
      end
      %w[on_macos on_linux on_arm on_intel].each { |b| assert_includes formula, "#{b} do" }
      assert_includes formula, 'bin.install Dir["lt-*"].first => "lt"'
      assert_includes formula, 'shell_output("#{bin}/lt --version")'
      assert_includes formula, 'conflicts_with "localtunnel"'
      refute_includes formula, token
      _o, rc = Open3.capture2e("ruby", "-c", "-e", formula.gsub(/^class Lt < Formula/, "class Formula; end\nclass Lt < Formula"))
      assert rc.success?, "the formula is not valid Ruby"
      assert_equal "class Ligus < Formula\nend\n", `git --git-dir=#{tap} show main:Formula/ligus.rb`,
                   "the other formula on the tap is untouched"
      assert_equal "github-actions[bot]", `git --git-dir=#{tap} log -1 --format=%an main`.strip
    end
  end
end

class MacosSigning < Minitest::Test
  SIGN = File.join(ROOT, "scripts/macos-sign.sh")

  def test_signing_uses_lts_identifier
    src = File.read(File.join(ROOT, "src/backend/real/mod.rs"))
    id = src[/pub const SIGNING_IDENTIFIER: &str = "([^"]+)"/, 1]
    assert_equal "ai.ligus.term", id
    assert_includes File.read(SIGN), %(IDENTIFIER="#{id}")
    assert_includes File.read(SIGN), '--identifier "$IDENTIFIER"'
    assert_includes File.read(SIGN), "--options runtime"
  end

  def test_configure_runs_before_the_build_and_signing_after
    st = steps(RELEASE, "build")
    configure = st.index { |s| s["run"].to_s.include?("macos-sign.sh configure") }
    build = st.index { |s| s["uses"] == "./.github/actions/build-lt" }
    sign = st.index { |s| s["run"].to_s.include?("macos-sign.sh sign") }
    notarize = st.index { |s| s["run"].to_s.include?("macos-sign.sh notarize") }
    stage = st.index { |s| s["name"] == "Stage asset" }
    assert_operator configure, :<, build, "LIGUS_APPLE_TEAM_ID must be set before the build"
    assert_operator build, :<, sign
    assert_operator sign, :<, notarize
    assert_operator notarize, :<, stage, "the checksum must be of the signed binary"
    [sign, notarize].each { |i| assert_includes st[i]["if"], "env.LT_MACOS_SIGNED == '1'" }
    cleanup = st.find { |s| s["run"].to_s.include?("macos-sign.sh cleanup") }
    assert_includes cleanup["if"], "always()"
  end

  def test_skips_cleanly_without_the_certificate
    Dir.mktmpdir do |dir|
      genv = File.join(dir, "env")
      File.write(genv, "")
      out, st = Open3.capture2e({ "APPLE_CERTIFICATE" => "", "GITHUB_ENV" => genv, "RUNNER_TEMP" => dir }, "bash", SIGN, "configure")
      assert st.success?, out
      assert_includes out, "unsigned"
      assert_equal "", File.read(genv), "an unsigned build must not get LIGUS_APPLE_TEAM_ID"
    end
  end

  def test_a_half_configured_release_fails
    Dir.mktmpdir do |dir|
      genv = File.join(dir, "env")
      File.write(genv, "")
      out, st = Open3.capture2e({ "APPLE_CERTIFICATE" => "x", "APPLE_TEAM_ID" => "", "GITHUB_ENV" => genv, "RUNNER_TEMP" => dir },
                                "bash", SIGN, "configure")
      refute st.success?
      assert_includes out, "APPLE_TEAM_ID"
      assert_equal "", File.read(genv)
    end
  end

  def test_secrets_reach_only_the_macos_signing_steps
    steps(RELEASE, "build").each do |s|
      next unless JSON.dump(s.fetch("env", {})).include?("secrets.APPLE_")

      assert_includes s["if"].to_s, "runner.os == 'macOS'", "#{s['name']} gets Apple secrets on Linux"
      assert_match(/macos-sign\.sh/, s["run"].to_s)
    end
  end
end

# scripts/check-release-config.sh on fixture trees.
class ReleaseConfig < Minitest::Test
  SCRIPT = File.join(ROOT, "scripts/check-release-config.sh")

  def run_check(tag, toml_version, lock_version)
    Dir.mktmpdir do |dir|
      File.write(File.join(dir, "Cargo.toml"), %([package]\nname = "ligus-term"\nversion = "#{toml_version}"\n\n[dependencies]\nfoo = { version = "9.9.9" }\n))
      File.write(File.join(dir, "Cargo.lock"), %([[package]]\nname = "foo"\nversion = "9.9.9"\n\n[[package]]\nname = "ligus-term"\nversion = "#{lock_version}"\n))
      out, st = Open3.capture2e({ "LT_RELEASE_ROOT" => dir }, "bash", SCRIPT, tag)
      [st.success?, out]
    end
  end

  def test_matching_versions_pass
    ok, out = run_check("v0.1.1", "0.1.1", "0.1.1")
    assert ok, out
  end

  def test_mismatches_fail
    ok, out = run_check("v0.1.1", "0.1.0", "0.1.1")
    refute ok
    assert_includes out, "Cargo.toml"
    ok, out = run_check("v0.1.1", "0.1.1", "0.1.0")
    refute ok
    assert_includes out, "Cargo.lock"
  end

  def test_malformed_tags_fail
    %w[0.1.1 v0.1 v0.1.1/x latest].each do |t|
      ok, = run_check(t, "0.1.1", "0.1.1")
      refute ok, t
    end
  end

  def test_the_repo_itself_is_consistent
    toml = File.read(File.join(ROOT, "Cargo.toml"))[/^\[package\].*?^version = "([^"]+)"/m, 1]
    out, st = Open3.capture2e("bash", SCRIPT, "v#{toml}")
    assert st.success?, out
  end
end

# scripts/check-release-assets.py on fixture drafts.
class CheckReleaseAssets < Minitest::Test
  SCRIPT = File.join(ROOT, "scripts/check-release-assets.py")
  HASH = "a" * 64

  def draft
    assets = []
    sums = {}
    TARGETS.each do |t|
      assets << { "name" => "lt-#{t}", "state" => "uploaded", "digest" => "sha256:#{HASH}" }
      assets << { "name" => "lt-#{t}.sha256", "state" => "uploaded" }
      sums["lt-#{t}.sha256"] = "#{HASH}  lt-#{t}\n"
    end
    [assets, sums]
  end

  def check(assets, sums)
    Dir.mktmpdir do |dir|
      File.write(File.join(dir, "assets.json"), JSON.dump(assets))
      sums.each { |n, v| File.write(File.join(dir, n), v) }
      out, _err, status = Open3.capture3("python3", "-I", SCRIPT, "--from-dir", dir, "--tag", "v0.1.1")
      [status.success?, out]
    end
  end

  def test_complete_draft_passes
    ok, out = check(*draft)
    assert ok, out
  end

  def test_missing_target_fails
    assets, sums = draft
    assets.reject! { |a| a["name"] == "lt-aarch64-unknown-linux-musl" }
    ok, out = check(assets, sums)
    refute ok
    assert_includes out, "lt-aarch64-unknown-linux-musl: missing"
  end

  def test_missing_checksum_fails
    assets, sums = draft
    assets.reject! { |a| a["name"] == "lt-x86_64-apple-darwin.sha256" }
    ok, out = check(assets, sums)
    refute ok
    assert_includes out, "lt-x86_64-apple-darwin.sha256: missing"
  end

  def test_checksum_that_does_not_match_the_digest_fails
    assets, sums = draft
    sums["lt-x86_64-unknown-linux-musl.sha256"] = "#{'b' * 64}  lt-x86_64-unknown-linux-musl\n"
    ok, out = check(assets, sums)
    refute ok
    assert_includes out, "differs"
  end

  def test_checksum_for_another_file_fails
    assets, sums = draft
    sums["lt-aarch64-apple-darwin.sha256"] = "#{HASH}  lt-x86_64-apple-darwin\n"
    ok, out = check(assets, sums)
    refute ok
    assert_includes out, "not a sha256 line"
  end

  def test_unfinished_upload_fails
    assets, sums = draft
    assets.find { |a| a["name"] == "lt-aarch64-apple-darwin" }["state"] = "starter"
    ok, out = check(assets, sums)
    refute ok
    assert_includes out, "upload not finished"
  end
end

# cut-release.yml: the normal way to release, from GitHub, never a laptop.
class CutRelease < Minitest::Test
  def cut_steps
    steps(CUT, "cut")
  end

  def idx(text)
    i = cut_steps.index { |s| s["run"].to_s.include?(text) || s["uses"].to_s.include?(text) }
    refute_nil i, "cut-release has no step with #{text}"
    i
  end

  def test_dispatch_only_with_a_version
    assert_equal ["workflow_dispatch"], triggers(CUT).keys
    inputs = triggers(CUT)["workflow_dispatch"]["inputs"]
    assert inputs["version"]["required"]
    refute inputs["ref"]["required"]
  end

  def test_least_privilege
    assert_equal({}, CUT["permissions"], "nothing by default")
    assert_equal({ "contents" => "write", "checks" => "read" }, CUT.dig("jobs", "cut", "permissions"))
    assert_equal({ "contents" => "write" }, CUT.dig("jobs", "release", "permissions"))
  end

  def test_order_of_checks_before_the_tag
    admin = idx("collaborators/$who/permission")
    sha = idx("repos/$GITHUB_REPOSITORY/commits/$ref")
    ci = idx("check_name=ci-ok")
    config = idx("check-release-config.sh")
    tag = idx("git tag -a")
    assert_operator admin, :<, sha
    assert_operator sha, :<, ci
    assert_operator ci, :<, config
    assert_operator config, :<, tag
    tag_run = cut_steps[tag]["run"]
    assert_operator tag_run.index("git ls-remote --tags"), :<, tag_run.index("git tag -a"), "refuse an existing tag first"
    refute_match(/push.*(-f|--force)/, tag_run)
  end

  def test_admins_only_including_reruns
    run = cut_steps[idx("collaborators/$who/permission")]
    assert_includes run["run"], '[ "$perm" != admin ]'
    assert_equal "${{ github.actor }}", run.dig("env", "ACTOR")
    assert_equal "${{ github.triggering_actor }}", run.dig("env", "TRIGGERING_ACTOR")
  end

  def test_ci_ok_must_have_passed_on_that_exact_commit
    st = cut_steps[idx("check_name=ci-ok")]
    assert_equal "${{ steps.sha.outputs.sha }}", st.dig("env", "SHA")
    assert_includes st["run"], "commits/$SHA/check-runs"
    assert_includes st["run"], '.conclusion == "success"'
    assert_includes st["run"], '.app.slug == "github-actions"'
    checkout = cut_steps[idx("actions/checkout@")]
    assert_equal "${{ steps.sha.outputs.sha }}", checkout.dig("with", "ref"), "the checks and the tag must be on one commit"
    assert_includes cut_steps[idx("git tag -a")]["run"], '"$SHA"'
    assert CI.fetch("jobs").key?("ci-ok"), "cut-release waits for a ci-ok job that ci.yml doesn't have"
  end

  def test_pipeline_is_called_once_with_the_tag
    rel = CUT.dig("jobs", "release")
    assert_equal "./.github/workflows/release.yml", rel["uses"]
    assert_equal "${{ needs.cut.outputs.tag }}", rel.dig("with", "tag")
    assert_equal ["cut"], Array(rel["needs"])
    assert_equal "inherit", rel["secrets"]
    checkout = cut_steps[idx("actions/checkout@")]
    refute checkout.fetch("with", {}).key?("token"), "a PAT/App token would make the tag push start release.yml too"
    refute_includes JSON.dump(CUT), "secrets.HOMEBREW", "cut-release itself needs no other secrets"
  end
end

# A script a workflow runs as `scripts/x.sh` must be executable in git, or the
# runner stops with "Permission denied" (v0.1.1's first cut did, on macOS).
class ScriptsAreExecutable < Minitest::Test
  def test_every_script_a_workflow_runs_directly_is_executable_in_git
    yml = Dir[File.join(ROOT, ".github/{workflows,actions}/**/*.yml")].map { |f| File.read(f) }.join("\n")
    # Commands only: `run: scripts/x.sh …` or a run-block line starting with it
    # (not `bash scripts/…`, and not a `- scripts/…` paths-filter entry).
    run = yml.scan(%r{^\s*(?:run:\s*)?(scripts/[\w.-]+\.(?:sh|py|rb))\b}).flatten.uniq
    refute_empty run
    modes = Open3.capture2("git", "-C", ROOT, "ls-files", "-s", *run).first
    run.each do |s|
      mode = modes.lines.find { |l| l.split("\t").last.strip == s }&.split&.first
      assert_equal "100755", mode, "#{s} is run directly by a workflow but isn't executable in git (git update-index --chmod=+x #{s})"
    end
  end
end

# The decoded certificate lands in an extensionless temp file, so `security
# import` must be told it's PKCS#12 ("Unknown format in import" otherwise;
# v0.1.1's second cut failed on it).
class CertificateImport < Minitest::Test
  def test_the_certificate_is_imported_as_pkcs12
    sign = File.read(File.join(ROOT, "scripts/macos-sign.sh"))
    imports = sign.lines.grep(/security import/)
    refute_empty imports
    imports.each { |l| assert_match(/-f pkcs12/, l, "security import needs -f pkcs12: #{l.strip}") }
  end
end
