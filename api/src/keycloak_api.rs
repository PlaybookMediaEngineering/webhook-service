use log::{error, trace};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::{Client, Response, Url};
use serde::{Deserialize, Serialize};
use strum::IntoEnumIterator;
use uuid::Uuid;

use crate::iam::{Role, GROUP_SEP, ORGA_GROUP_PREFIX, ROLE_GROUP_PREFIX};
use crate::problems::Hook0Problem;

#[derive(Debug, Clone)]
pub struct KeycloakApi {
    client: Client,
    api_url: Url,
    #[allow(dead_code)]
    access_token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct KcGroup<'a> {
    name: &'a str,
    path: &'a str,
    #[serde(skip)]
    parent: Option<&'a Uuid>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct KcUser<'a> {
    username: &'a str,
    email: &'a str,
    first_name: &'a str,
    last_name: &'a str,
    credentials: Vec<KcUserCredential<'a>>,
    enabled: bool,
    email_verified: bool,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct KcUserCredential<'a> {
    id: &'a str,
    value: &'a str,
    temporary: bool,
}

impl KeycloakApi {
    pub async fn new(
        keycloak_url: &Url,
        keycloak_realm: &str,
        client_id: &str,
        client_secret: &str,
    ) -> Result<Self, Hook0Problem> {
        let url = append_url_segments(
            keycloak_url,
            &[
                "realms",
                keycloak_realm,
                "protocol",
                "openid-connect",
                "token",
            ],
        )
        .map_err(|e| {
            error!(
                "Could not create a valid URL to request Keycloak's API: {}",
                &e
            );
            Hook0Problem::InternalServerError
        })?;

        let body = [
            ("grant_type", "client_credentials"),
            ("client_id", client_id),
            ("client_secret", client_secret),
        ];
        #[derive(Debug, Deserialize)]
        struct AuthResponse {
            access_token: String,
        }
        const OPERATION: &str = "getting an access token from Keycloak";

        trace!(
            "Requesting an access token to Keycloak (client_id = {})",
            client_id
        );

        let res = Client::new()
            .post(url)
            .form(&body)
            .send()
            .await
            .map_err(|e| {
                error!("Error while {}: {}", OPERATION, &e);
                Hook0Problem::InternalServerError
            })?
            .error_for_status()
            .map_err(|e| {
                error!("Error while {}: {}", OPERATION, &e);
                Hook0Problem::InternalServerError
            })?
            .json::<AuthResponse>()
            .await
            .map_err(|e| {
                error!("Error while {}: {}", OPERATION, &e);
                Hook0Problem::InternalServerError
            })?;

        let authenticated_client = HeaderValue::from_str(&format!("Bearer {}", &res.access_token))
            .map_err(|e| {
                error!("Could not build auth header: {}", &e);
                Hook0Problem::InternalServerError
            })
            .map(|hv| HeaderMap::from_iter([(AUTHORIZATION, hv)]))
            .and_then(|headers| {
                Client::builder()
                    .default_headers(headers)
                    .build()
                    .map_err(|e| {
                        error!("Could not build HTTP client: {}", &e);
                        Hook0Problem::InternalServerError
                    })
            })?;
        let api_url = append_url_segments(keycloak_url, &["admin", "realms", keycloak_realm])
            .map_err(|e| {
                error!(
                    "Could not create a valid URL to request Keycloak's API: {}",
                    &e
                );
                Hook0Problem::InternalServerError
            })?;

        Ok(Self {
            client: authenticated_client,
            api_url,
            access_token: res.access_token,
        })
    }

    fn mk_url(&self, segments: &[&str]) -> Result<Url, Hook0Problem> {
        append_url_segments(&self.api_url, segments).map_err(|e| {
            error!(
                "Could not create a valid URL to request Keycloak's API: {}",
                &e
            );
            Hook0Problem::InternalServerError
        })
    }

    pub async fn create_organization(&self, organization_id: &Uuid) -> Result<Uuid, Hook0Problem> {
        let main_group = KcGroup {
            name: &format!("{}{}", ORGA_GROUP_PREFIX, &organization_id),
            path: &format!("{}{}{}", GROUP_SEP, ORGA_GROUP_PREFIX, &organization_id),
            parent: None,
        };

        trace!("Creating organization groups in Keycloak");
        let main_group_id = self.create_group(&main_group).await?;
        trace!("Main group created (ID={})", &main_group_id);

        let mut editor_group_id = None;
        for role in Role::iter() {
            let group = KcGroup {
                name: &format!("{}{}", ROLE_GROUP_PREFIX, &role),
                path: &format!(
                    "{}{}{}{}{}{}",
                    GROUP_SEP,
                    ORGA_GROUP_PREFIX,
                    &organization_id,
                    GROUP_SEP,
                    ROLE_GROUP_PREFIX,
                    &role
                ),
                parent: Some(&main_group_id),
            };

            let id = self.create_group(&group).await?;
            if role == Role::Editor {
                editor_group_id = Some(id);
            }
        }

        let editor_group_id = editor_group_id.ok_or_else(|| {
            error!("Could not get ID of the newly created editor group");
            Hook0Problem::InternalServerError
        })?;

        Ok(editor_group_id)
    }

    async fn create_group(&self, group: &KcGroup<'_>) -> Result<Uuid, Hook0Problem> {
        let operation = format!("creating '{}' group in Keycloak", &group.path);
        let group_url = if let Some(parent_group_id) = group.parent {
            self.mk_url(&["groups", parent_group_id.to_string().as_str(), "children"])?
        } else {
            self.mk_url(&["groups"])?
        };

        let res = self
            .client
            .post(group_url.as_str())
            .json(&group)
            .send()
            .await
            .map_err(|e| {
                error!("Error while {}: {}", operation, &e);
                Hook0Problem::InternalServerError
            })?
            .error_for_status()
            .map_err(|e| {
                error!("Error while {}: {}", operation, &e);
                Hook0Problem::InternalServerError
            })?;

        let id = Self::extract_resource_id_from_redirection(&res)?;
        Ok(id)
    }

    pub async fn create_user(
        &self,
        email: &str,
        first_name: &str,
        last_name: &str,
        editor_group_id: Option<&Uuid>,
    ) -> Result<(Uuid, String), Hook0Problem> {
        let user_url = self.mk_url(&["users"])?;
        let password = Self::gen_password();
        let user = KcUser {
            username: email,
            email,
            first_name,
            last_name,
            credentials: vec![KcUserCredential {
                id: "password",
                value: &password,
                temporary: true, // TODO: send a welcome email and set this to true
            }],
            enabled: true,
            email_verified: false,
        };
        const OPERATION: &str = "creating user in Keycloak";

        trace!("Creating user in Keycloak");

        let res = self
            .client
            .post(user_url.as_str())
            .json(&user)
            .send()
            .await
            .map_err(|e| {
                error!("Error while {}: {}", OPERATION, &e);
                Hook0Problem::InternalServerError
            })?
            .error_for_status()
            .map_err(|e| {
                error!("Error while {}: {}", OPERATION, &e);
                Hook0Problem::InternalServerError
            })?;
        let user_id = Self::extract_resource_id_from_redirection(&res)?;

        if let Some(group_id) = editor_group_id {
            trace!("Assigning group to user in Keycloak");

            let user_group_url = self.mk_url(&[
                "users",
                user_id.to_string().as_str(),
                "groups",
                group_id.to_string().as_str(),
            ])?;

            self.client
                .put(user_group_url.as_str())
                .send()
                .await
                .map_err(|e| {
                    error!("Error while {}: {}", OPERATION, &e);
                    Hook0Problem::InternalServerError
                })?
                .error_for_status()
                .map_err(|e| {
                    error!("Error while {}: {}", OPERATION, &e);
                    Hook0Problem::InternalServerError
                })?;
        }

        Ok((user_id, password))
    }

    pub async fn ensure_email_does_not_exist(&self, user_email: &str) -> Result<(), Hook0Problem> {
        let url = self.mk_url(&["users", "count"])?;
        let query = [("email", user_email)];
        const OPERATION: &str = "checking if user already exists in Keycloak";

        trace!("Checking if a Keycloak user with this email address already exists");

        let res = self
            .client
            .get(url)
            .query(&query)
            .send()
            .await
            .map_err(|e| {
                error!("Error while {}: {}", OPERATION, &e);
                Hook0Problem::InternalServerError
            })?
            .error_for_status()
            .map_err(|e| {
                error!("Error while {}: {}", OPERATION, &e);
                Hook0Problem::InternalServerError
            })?
            .text()
            .await
            .map_err(|e| {
                error!("Error while {}: {}", OPERATION, &e);
                Hook0Problem::InternalServerError
            })?;

        let count = res.parse::<i32>();

        match count {
            Ok(c) if c == 0 => Ok(()),
            Ok(c) if c < 0 => {
                error!(
                    "Error while parsing result given by Keycloak API: search found {} users",
                    &c
                );
                Err(Hook0Problem::InternalServerError)
            }
            Ok(_) => Err(Hook0Problem::UserAlreadyExist),
            Err(e) => {
                error!("Error while parsing result given by Keycloak API: {}", &e);
                Err(Hook0Problem::InternalServerError)
            }
        }
    }

    fn extract_resource_id_from_redirection(res: &Response) -> Result<Uuid, Hook0Problem> {
        res.headers()
            .get("location")
            .ok_or_else(|| {
                error!("Could not find the Location header in the response");
                Hook0Problem::InternalServerError
            })
            .and_then(|hv| {
                hv.to_str().map_err(|e| {
                    error!("Could not convert response header to string: {}", &e);
                    Hook0Problem::InternalServerError
                })
            })
            .and_then(|location| {
                Url::parse(location).map_err(|e| {
                    error!("Could not parse Location header as a URL: {}", &e);
                    Hook0Problem::InternalServerError
                })
            })
            .and_then(|url| {
                url.path_segments()
                    .ok_or_else(|| {
                        error!("Could not split Location header's URL into segments",);
                        Hook0Problem::InternalServerError
                    })
                    .and_then(|segments| {
                        segments.last().map(|s| s.to_owned()).ok_or_else(|| {
                            error!("Could not get last segment of Location header's URL");
                            Hook0Problem::InternalServerError
                        })
                    })
            })
            .and_then(|last| {
                Uuid::parse_str(&last).map_err(|e| {
                    error!(
                        "Could not parse last segment of Location header's URL into UUID: {}",
                        &e
                    );
                    Hook0Problem::InternalServerError
                })
            })
    }

    fn gen_password() -> String {
        use rand::Rng;

        rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(15)
            .map(|byte| byte as char)
            .collect()
    }
}

pub fn append_url_segments(base_url: &Url, segments: &[&str]) -> Result<Url, url::ParseError> {
    const SEP: &str = "/";
    let segments_str = segments.join(SEP);

    let url = base_url.join(&format!("{}/{}", base_url.path(), segments_str))?;

    Ok(url)
}