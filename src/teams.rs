// TODO maybe pull from https://github.com/rust-lang/rust-www/blob/master/_data/team.yml instead

use std::collections::BTreeMap;

use diesel::prelude::*;
use toml;
use arrayvec::ArrayVec;

use super::DB_POOL;
use domain::github::{GitHubUser};
use github::models::Reaction;
use error::*;

//==============================================================================
// Public API
//==============================================================================

lazy_static! {
    pub static ref SETUP: RfcbotConfig = read_rfcbot_cfg_validated();
}

#[derive(Debug, Deserialize)]
pub struct RfcbotConfig {
    prohibited_reactions: BTreeMap<String, ReactionBehaviorConfig>,
    fcp_behaviors: BTreeMap<String, FcpBehavior>,
    teams: BTreeMap<TeamLabel, Team>,
}

impl RfcbotConfig {
    /// Retrive an iterator over all the team labels.
    pub fn team_labels(&self) -> impl Iterator<Item = &TeamLabel> {
        self.teams.keys()
    }

    /// Retrive an iterator over all the (team label, team) pairs.
    pub fn teams(&self) -> impl Iterator<Item = (&TeamLabel, &Team)> {
        self.teams.iter()
    }

    /// Are we allowed to auto-close issues after F-FCP in this repo?
    pub fn should_ffcp_auto_close(&self, repo: &str) -> bool {
        self.fcp_behaviors.get(repo).map(|fcp| fcp.close).unwrap_or_default()
    }

    /// Are we allowed to auto-postpone issues after F-FCP in this repo?
    pub fn should_ffcp_auto_postpone(&self, repo: &str) -> bool {
        self.fcp_behaviors.get(repo).map(|fcp| fcp.postpone).unwrap_or_default()
    }

    /// Returns all the prohibited reactions on issues for this repo.
    pub fn prohibited_issue_reactions(&self, repo: &str) -> ReactionVec {
        self.prohibited_reactions.get(repo)
            .map(|rb| rb.issue.prohibited_reactions())
            .unwrap_or_default()
    }

    /// Returns all the prohibited reactions on comments for this repo.
    pub fn prohibited_comment_reactions(&self, repo: &str) -> ReactionVec {
        self.prohibited_reactions.get(repo)
            .map(|rb| rb.comment.prohibited_reactions())
            .unwrap_or_default()
    }
}

#[derive(Copy, Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct ReactionBehaviorConfig {
    issue: ProhibitedReactions,
    comment: ProhibitedReactions,
}

#[derive(Copy, Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct ProhibitedReactions {
    up_vote: bool,
    down_vote: bool,
    laugh: bool,
    hooray: bool,
    confused: bool,
    heart: bool,
}

type ReactionVec = ArrayVec<[Reaction; 6]>;

impl ProhibitedReactions {
    fn prohibited_reactions(&self) -> ReactionVec {
        use self::Reaction::*;
        [Upvote, Downvote, Laugh, Hooray, Confused, Heart].iter()
            .filter(move |&&reaction| self.is_reaction_prohibited(reaction))
            .cloned()
            .collect()
    }

    fn is_reaction_prohibited(&self, reaction: Reaction) -> bool {
        use self::Reaction::*;
        match reaction {
            Upvote => self.up_vote,
            Downvote => self.down_vote,
            Laugh => self.laugh,
            Hooray => self.hooray,
            Confused => self.confused,
            Heart => self.heart,
        }
    }
}

#[derive(Debug, Deserialize)]
struct FcpBehavior {
    #[serde(default)]
    close: bool,
    #[serde(default)]
    postpone: bool,
}

#[derive(Debug, Deserialize)]
pub struct Team {
    // FIXME(2018-05-16):
    // The two following first fields are not used anymore.
    // But they could still be useful. Consider what usage they could have.

    //name: String,
    //ping: String,

    members: Vec<String>,
}

impl Team {
    pub fn member_logins(&self) -> impl Iterator<Item = &str> {
        self.members.iter().map(|s| s.as_str())
    }
}

#[derive(Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize)]
#[serde(transparent)]
pub struct TeamLabel(pub String);

//==============================================================================
// Implementation details
//==============================================================================

/// Read the validated `rfcbot.toml` configuration file.
fn read_rfcbot_cfg_validated() -> RfcbotConfig {
    let cfg = read_rfcbot_cfg();

    cfg.teams.values().for_each(|team|
        team.validate()
            .expect("unable to verify team member from database.
if you're running this for tests, make sure you've pulled github users from prod")
    );

    cfg
}

/// Read the unprocessed `rfcbot.toml` configuration file.
fn read_rfcbot_cfg() -> RfcbotConfig {
    read_rfcbot_cfg_from(
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/rfcbot.toml")))
}

fn read_rfcbot_cfg_from(input: &str) -> RfcbotConfig {
    toml::from_str(input).expect("couldn't parse rfcbot.toml!")
}

impl Team {
    fn validate(&self) -> DashResult<()> {
        use domain::schema::githubuser::dsl::*;
        let conn = &*(DB_POOL.get()?);

        // bail if they don't exist, but we don't want to actually keep the id in ram
        for member_login in self.member_logins() {
            let check_login = githubuser.filter(login.eq(member_login))
                                        .first::<GitHubUser>(conn);
            ok_or!(check_login, why => {
                error!("unable to find {} in database: {:?}", member_login, why);
                throw!(why);
            });
        }

        Ok(())
    }
}

//==============================================================================
// Tests
//==============================================================================

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn setup_parser_correct() {
let test = r#"
[prohibited_reactions]

[prohibited_reactions."foo-org/bar".issue]
down_vote = true
confused = true

[prohibited_reactions."foo-org/bar".comment]
down_vote = true
confused = false

[fcp_behaviors]

[fcp_behaviors."rust-lang/alpha"]
close = true
postpone = true

[fcp_behaviors."foobar/beta"]
close = false

[fcp_behaviors."bazquux/gamma"]
postpone = false

[fcp_behaviors."wibble/epsilon"]

[teams]

[teams.avengers]
name = "The Avengers"
ping = "marvel/avengers"
members = [
  "hulk",
  "thor",
  "thevision",
  "blackwidow",
  "spiderman",
  "captainamerica",
]

[teams.justice-league]
name = "Justice League of America"
ping = "dc-comics/justice-league"
members = [
  "superman",
  "wonderwoman",
  "aquaman",
  "batman",
  "theflash"
]
"#;
        let cfg = read_rfcbot_cfg_from(test);

        // Labels are correct:
        assert_eq!(cfg.team_labels().map(|tl| tl.0.clone()).collect::<Vec<_>>(),
                   vec!["avengers", "justice-league"]);

        // Teams are correct:
        let map: BTreeMap<_, _> =
            cfg.teams().map(|(k, v)| (k.0.clone(), v.clone())).collect();

        let avengers = map.get("avengers").unwrap();
        //assert_eq!(avengers.name, "The Avengers");
        //assert_eq!(avengers.ping, "marvel/avengers");
        assert_eq!(avengers.member_logins().collect::<Vec<_>>(),
            vec!["hulk", "thor", "thevision", "blackwidow",
                 "spiderman", "captainamerica"]);

        let jsa = map.get("justice-league").unwrap();
        //assert_eq!(jsa.name, "Justice League of America");
        //assert_eq!(jsa.ping, "dc-comics/justice-league");
        assert_eq!(jsa.member_logins().collect::<Vec<_>>(),
            vec!["superman", "wonderwoman", "aquaman", "batman", "theflash"]);

        // Random non-existent team does not exist:
        assert!(map.get("random").is_none());

        // FFCP behavior correct:
        assert!(cfg.should_ffcp_auto_close("rust-lang/alpha"));
        assert!(cfg.should_ffcp_auto_postpone("rust-lang/alpha"));
        assert!(!cfg.should_ffcp_auto_close("foobar/beta"));
        assert!(!cfg.should_ffcp_auto_postpone("foobar/beta"));
        assert!(!cfg.should_ffcp_auto_close("bazquux/gamma"));
        assert!(!cfg.should_ffcp_auto_postpone("bazquux/gamma"));
        assert!(!cfg.should_ffcp_auto_close("wibble/epsilon"));
        assert!(!cfg.should_ffcp_auto_postpone("wibble/epsilon"));
        assert!(!cfg.should_ffcp_auto_close("random"));
        assert!(!cfg.should_ffcp_auto_postpone("random"));

        // Reaction behavior correct:
        let foo_issue_nono = cfg.prohibited_issue_reactions("foo-org/bar")
                                .into_iter().collect::<Vec<_>>();
        assert_eq!(foo_issue_nono, &[Reaction::Downvote, Reaction::Confused]);
        let foo_comment_nono = cfg.prohibited_comment_reactions("foo-org/bar")
                                  .into_iter().collect::<Vec<_>>();
        assert_eq!(foo_comment_nono, &[Reaction::Downvote]);
        assert!(cfg.prohibited_issue_reactions("random").is_empty());
        assert!(cfg.prohibited_comment_reactions("random").is_empty());
    }

    #[test]
    fn cfg_file_wellformed() {
        // Just parse it and ensure that we get no panics for now!
        // This is a crap test; but, better than nothing.
        let _ = read_rfcbot_cfg();
    }

    #[test]
    fn team_members_exist() {
        for (label, _) in SETUP.teams.iter() {
            println!("found team {:?}", label);
        }
    }
}
