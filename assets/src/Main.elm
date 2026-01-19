port module Main exposing (main)

import Browser
import Browser.Navigation as Nav
import Html exposing (Html, a, aside, button, div, footer, h1, h2, h3, header, input, label, main_, nav, p, pre, span, text)
import Html.Attributes exposing (class, disabled, for, href, id, placeholder, required, target, title, type_, value)
import Html.Events exposing (onClick, onInput, onSubmit)
import Http
import Json.Decode as Decode
import Json.Encode as Encode
import Url
import Url.Parser as Parser exposing ((</>), Parser, oneOf, s, string)


-- PORTS


port saveToken : String -> Cmd msg


port clearToken : () -> Cmd msg


port startGitHubSSE : { token : String, deviceCode : String, interval : Int, expiresIn : Int } -> Cmd msg


port gitHubSSEEvent : (Decode.Value -> msg) -> Sub msg



-- ROUTING


type Route
    = LoginRoute
    | SetupRoute
    | DashboardRoute
    | AppDetailRoute String


routeParser : Parser (Route -> a) a
routeParser =
    oneOf
        [ Parser.map LoginRoute Parser.top
        , Parser.map LoginRoute (s "login")
        , Parser.map SetupRoute (s "setup")
        , Parser.map DashboardRoute (s "dashboard")
        , Parser.map AppDetailRoute (s "apps" </> string)
        ]


fromUrl : Url.Url -> Maybe Route
fromUrl url =
    Parser.parse routeParser url


routeToString : Route -> String
routeToString route =
    case route of
        LoginRoute ->
            "/login"

        SetupRoute ->
            "/setup"

        DashboardRoute ->
            "/dashboard"

        AppDetailRoute appName ->
            "/apps/" ++ appName



-- MAIN


main : Program Flags Model Msg
main =
    Browser.application
        { init = init
        , update = update
        , view = view
        , subscriptions = subscriptions
        , onUrlChange = UrlChanged
        , onUrlRequest = LinkClicked
        }



-- FLAGS


type alias Flags =
    { token : Maybe String
    }



-- MODEL


type Page
    = Loading
    | Setup SetupForm
    | Login LoginForm
    | Dashboard DashboardState


type alias SetupForm =
    { email : String
    , password : String
    , confirmPassword : String
    , fullName : String
    , organizationName : String
    , error : Maybe String
    , submitting : Bool
    }


type alias LoginForm =
    { email : String
    , password : String
    , error : Maybe String
    , submitting : Bool
    }


type alias UserInfo =
    { email : String
    , fullName : String
    }


type SidebarItem
    = MyApps
    | Activity
    | Backups
    | Settings


type alias DashboardState =
    { user : UserInfo
    , view : DashboardView
    , apps : List AppInfo
    , appsLoading : Bool
    , githubStatus : GitHubStatus
    , token : String
    , activeSidebarItem : SidebarItem
    }


type DashboardView
    = AppsListView
    | CreateAppView CreateAppState
    | AppDetailView AppDetailState


type alias AppDetailState =
    { app : AppDetail
    , logs : String
    , logsLoading : Bool
    , actionInProgress : Maybe AppAction
    , error : Maybe String
    }


type alias AppDetail =
    { id : String
    , name : String
    , state : String
    , port_ : Maybe Int
    , createdAt : String
    , updatedAt : String
    , remote : Maybe RemoteInfo
    }


type alias RemoteInfo =
    { name : String
    , url : String
    , branch : String
    }


type AppAction
    = Starting
    | Stopping
    | Building
    | Deleting


type alias CreateAppState =
    { appName : String
    , step : CreateAppStep
    , error : Maybe String
    }


type CreateAppStep
    = EnterName
    | CheckingGitHub
    | ConnectGitHub GitHubConnectState
    | SelectRepo (List RepoInfo) String
    | Creating


type alias GitHubConnectState =
    { userCode : String
    , verificationUri : String
    , deviceCode : String
    , expiresIn : Int
    , interval : Int
    , polling : Bool
    }


type GitHubStatus
    = GitHubUnknown
    | GitHubNotConnected
    | GitHubConnected String


type alias AppInfo =
    { id : String
    , name : String
    , state : String
    }


type alias RepoInfo =
    { name : String
    , fullName : String
    , description : Maybe String
    , private : Bool
    , cloneUrl : String
    , defaultBranch : String
    }


type alias Model =
    { page : Page
    , serverVersion : String
    , navKey : Nav.Key
    , currentRoute : Maybe Route
    }


emptySetupForm : SetupForm
emptySetupForm =
    { email = ""
    , password = ""
    , confirmPassword = ""
    , fullName = ""
    , organizationName = ""
    , error = Nothing
    , submitting = False
    }


emptyLoginForm : LoginForm
emptyLoginForm =
    { email = ""
    , password = ""
    , error = Nothing
    , submitting = False
    }


emptyCreateAppState : CreateAppState
emptyCreateAppState =
    { appName = ""
    , step = EnterName
    , error = Nothing
    }


init : Flags -> Url.Url -> Nav.Key -> ( Model, Cmd Msg )
init flags url navKey =
    let
        route =
            fromUrl url

        initialModel =
            { page = Loading
            , serverVersion = ""
            , navKey = navKey
            , currentRoute = route
            }
    in
    case flags.token of
        Just token ->
            -- We have a token, verify it
            ( initialModel
            , verifyToken token
            )

        Nothing ->
            -- No token, check server status
            ( initialModel
            , checkServerStatus
            )



-- SUBSCRIPTIONS


subscriptions : Model -> Sub Msg
subscriptions model =
    case model.page of
        Dashboard state ->
            case state.view of
                CreateAppView createState ->
                    case createState.step of
                        ConnectGitHub ghState ->
                            if ghState.polling then
                                gitHubSSEEvent GotGitHubSSEEvent

                            else
                                Sub.none

                        _ ->
                            Sub.none

                _ ->
                    Sub.none

        _ ->
            Sub.none



-- UPDATE


type Msg
    = UrlChanged Url.Url
    | LinkClicked Browser.UrlRequest
    | GotServerStatus (Result Http.Error ServerStatus)
    | GotTokenVerification (Result Http.Error TokenVerificationResponse)
    | GotLoginResponse (Result Http.Error AuthResponse)
    | GotRegisterResponse (Result Http.Error AuthResponse)
      -- Setup form
    | SetupEmailChanged String
    | SetupPasswordChanged String
    | SetupConfirmPasswordChanged String
    | SetupFullNameChanged String
    | SetupOrganizationNameChanged String
    | SubmitSetup
      -- Login form
    | LoginEmailChanged String
    | LoginPasswordChanged String
    | SubmitLogin
      -- Dashboard
    | Logout
    | GotApps (Result Http.Error (List AppInfo))
    | GotGitHubStatus (Result Http.Error GitHubStatusResponse)
      -- Create app flow
    | ShowCreateApp
    | CancelCreateApp
    | AppNameChanged String
    | SubmitAppName
    | StartGitHubConnect
    | GotDeviceFlowStart (Result Http.Error DeviceFlowStartResponse)
    | GotGitHubSSEEvent Decode.Value
    | GotRepoList (Result Http.Error (List RepoInfo))
    | RepoSearchChanged String
    | ChooseRepo RepoInfo
    | SkipRepoSelection
    | GotAppCreated (Result Http.Error AppInfo)
      -- App detail
    | ViewAppDetail String
    | GotAppDetail (Result Http.Error AppDetail)
    | BackToApps
    | RefreshAppDetail
    | StartApp
    | StopApp
    | BuildApp
    | DeleteApp
    | ConfirmDeleteApp
    | CancelDeleteApp
    | GotAppStarted (Result Http.Error String)
    | GotAppStopped (Result Http.Error String)
    | GotAppBuilt (Result Http.Error String)
    | GotAppDeleted (Result Http.Error String)
    | FetchLogs
    | GotLogs (Result Http.Error String)


type alias ServerStatus =
    { initialized : Bool
    , version : String
    }


type alias AuthResponse =
    { accessToken : String
    , user : UserInfo
    }


type alias TokenVerificationResponse =
    { user : UserInfo
    , token : String
    }


type alias GitHubStatusResponse =
    { connected : Bool
    , username : Maybe String
    }


type alias DeviceFlowStartResponse =
    { userCode : String
    , verificationUri : String
    , deviceCode : String
    , expiresIn : Int
    , interval : Int
    }




update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case msg of
        UrlChanged url ->
            let
                route =
                    fromUrl url
            in
            case route of
                Just LoginRoute ->
                    ( { model
                        | currentRoute = route
                        , page = Login emptyLoginForm
                      }
                    , Cmd.none
                    )

                Just SetupRoute ->
                    ( { model
                        | currentRoute = route
                        , page = Setup emptySetupForm
                      }
                    , Cmd.none
                    )

                Just DashboardRoute ->
                    case model.page of
                        Dashboard state ->
                            -- Already on dashboard, just switch to apps list view
                            ( { model
                                | currentRoute = route
                                , page = Dashboard { state | view = AppsListView }
                              }
                            , Cmd.none
                            )

                        _ ->
                            -- Not on dashboard, stay on current page (need to be logged in first)
                            ( { model | currentRoute = route }
                            , Cmd.none
                            )

                Just (AppDetailRoute appName) ->
                    case model.page of
                        Dashboard state ->
                            -- Fetch and show app detail
                            ( { model | currentRoute = route }
                            , fetchAppDetail state.token appName
                            )

                        _ ->
                            -- Not on dashboard, stay on current page
                            ( { model | currentRoute = route }
                            , Cmd.none
                            )

                Nothing ->
                    -- Unknown route, stay on current page
                    ( { model | currentRoute = route }
                    , Cmd.none
                    )

        LinkClicked urlRequest ->
            case urlRequest of
                Browser.Internal url ->
                    ( model
                    , Nav.pushUrl model.navKey (Url.toString url)
                    )

                Browser.External href ->
                    ( model
                    , Nav.load href
                    )

        GotServerStatus result ->
            case result of
                Ok status ->
                    if status.initialized then
                        ( { model
                            | page = Login emptyLoginForm
                            , serverVersion = status.version
                          }
                        , Nav.pushUrl model.navKey "/login"
                        )

                    else
                        ( { model
                            | page = Setup emptySetupForm
                            , serverVersion = status.version
                          }
                        , Nav.pushUrl model.navKey "/setup"
                        )

                Err _ ->
                    ( { model | page = Login { emptyLoginForm | error = Just "Failed to connect to server" } }
                    , Nav.pushUrl model.navKey "/login"
                    )

        GotTokenVerification result ->
            case result of
                Ok response ->
                    let
                        dashboardState =
                            { user = response.user
                            , view = AppsListView
                            , apps = []
                            , appsLoading = True
                            , githubStatus = GitHubUnknown
                            , token = response.token
                            , activeSidebarItem = MyApps
                            }
                    in
                    ( { model | page = Dashboard dashboardState }
                    , Cmd.batch
                        [ Nav.pushUrl model.navKey "/dashboard"
                        , fetchApps response.token
                        , fetchGitHubStatus response.token
                        ]
                    )

                Err _ ->
                    -- Token invalid, clear it and check status
                    ( { model | page = Loading }
                    , Cmd.batch [ clearToken (), checkServerStatus ]
                    )

        GotLoginResponse result ->
            case model.page of
                Login form ->
                    case result of
                        Ok response ->
                            let
                                dashboardState =
                                    { user = response.user
                                    , view = AppsListView
                                    , apps = []
                                    , appsLoading = True
                                    , githubStatus = GitHubUnknown
                                    , token = response.accessToken
                                    , activeSidebarItem = MyApps
                                    }
                            in
                            ( { model | page = Dashboard dashboardState }
                            , Cmd.batch
                                [ Nav.pushUrl model.navKey "/dashboard"
                                , saveToken response.accessToken
                                , fetchApps response.accessToken
                                , fetchGitHubStatus response.accessToken
                                ]
                            )

                        Err err ->
                            ( { model | page = Login { form | error = Just (httpErrorToString err), submitting = False } }
                            , Cmd.none
                            )

                _ ->
                    ( model, Cmd.none )

        GotRegisterResponse result ->
            case model.page of
                Setup form ->
                    case result of
                        Ok response ->
                            let
                                dashboardState =
                                    { user = response.user
                                    , view = AppsListView
                                    , apps = []
                                    , appsLoading = True
                                    , githubStatus = GitHubUnknown
                                    , token = response.accessToken
                                    , activeSidebarItem = MyApps
                                    }
                            in
                            ( { model | page = Dashboard dashboardState }
                            , Cmd.batch
                                [ Nav.pushUrl model.navKey "/dashboard"
                                , saveToken response.accessToken
                                , fetchApps response.accessToken
                                , fetchGitHubStatus response.accessToken
                                ]
                            )

                        Err err ->
                            ( { model | page = Setup { form | error = Just (httpErrorToString err), submitting = False } }
                            , Cmd.none
                            )

                _ ->
                    ( model, Cmd.none )

        -- Setup form messages
        SetupEmailChanged email ->
            case model.page of
                Setup form ->
                    ( { model | page = Setup { form | email = email } }, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        SetupPasswordChanged password ->
            case model.page of
                Setup form ->
                    ( { model | page = Setup { form | password = password } }, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        SetupConfirmPasswordChanged confirmPassword ->
            case model.page of
                Setup form ->
                    ( { model | page = Setup { form | confirmPassword = confirmPassword } }, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        SetupFullNameChanged fullName ->
            case model.page of
                Setup form ->
                    ( { model | page = Setup { form | fullName = fullName } }, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        SetupOrganizationNameChanged orgName ->
            case model.page of
                Setup form ->
                    ( { model | page = Setup { form | organizationName = orgName } }, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        SubmitSetup ->
            case model.page of
                Setup form ->
                    if form.password /= form.confirmPassword then
                        ( { model | page = Setup { form | error = Just "Passwords do not match" } }
                        , Cmd.none
                        )

                    else if String.length form.password < 8 then
                        ( { model | page = Setup { form | error = Just "Password must be at least 8 characters" } }
                        , Cmd.none
                        )

                    else
                        ( { model | page = Setup { form | submitting = True, error = Nothing } }
                        , submitRegister form
                        )

                _ ->
                    ( model, Cmd.none )

        -- Login form messages
        LoginEmailChanged email ->
            case model.page of
                Login form ->
                    ( { model | page = Login { form | email = email } }, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        LoginPasswordChanged password ->
            case model.page of
                Login form ->
                    ( { model | page = Login { form | password = password } }, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        SubmitLogin ->
            case model.page of
                Login form ->
                    ( { model | page = Login { form | submitting = True, error = Nothing } }
                    , submitLogin form
                    )

                _ ->
                    ( model, Cmd.none )

        Logout ->
            ( { model | page = Login emptyLoginForm }
            , Cmd.batch
                [ Nav.pushUrl model.navKey "/login"
                , clearToken ()
                ]
            )

        -- Dashboard messages
        GotApps result ->
            case model.page of
                Dashboard state ->
                    case result of
                        Ok apps ->
                            ( { model | page = Dashboard { state | apps = apps, appsLoading = False } }
                            , Cmd.none
                            )

                        Err _ ->
                            ( { model | page = Dashboard { state | appsLoading = False } }
                            , Cmd.none
                            )

                _ ->
                    ( model, Cmd.none )

        GotGitHubStatus result ->
            case model.page of
                Dashboard state ->
                    case result of
                        Ok response ->
                            let
                                status =
                                    if response.connected then
                                        case response.username of
                                            Just username ->
                                                GitHubConnected username

                                            Nothing ->
                                                GitHubConnected ""

                                    else
                                        GitHubNotConnected
                            in
                            ( { model | page = Dashboard { state | githubStatus = status } }
                            , Cmd.none
                            )

                        Err _ ->
                            ( { model | page = Dashboard { state | githubStatus = GitHubNotConnected } }
                            , Cmd.none
                            )

                _ ->
                    ( model, Cmd.none )

        -- Create app flow
        ShowCreateApp ->
            case model.page of
                Dashboard state ->
                    ( { model | page = Dashboard { state | view = CreateAppView emptyCreateAppState } }
                    , Cmd.none
                    )

                _ ->
                    ( model, Cmd.none )

        CancelCreateApp ->
            case model.page of
                Dashboard state ->
                    ( { model | page = Dashboard { state | view = AppsListView } }
                    , Cmd.none
                    )

                _ ->
                    ( model, Cmd.none )

        AppNameChanged name ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        CreateAppView createState ->
                            ( { model
                                | page =
                                    Dashboard
                                        { state
                                            | view = CreateAppView { createState | appName = name }
                                        }
                              }
                            , Cmd.none
                            )

                        _ ->
                            ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        SubmitAppName ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        CreateAppView createState ->
                            if String.isEmpty (String.trim createState.appName) then
                                ( { model
                                    | page =
                                        Dashboard
                                            { state
                                                | view = CreateAppView { createState | error = Just "App name is required" }
                                            }
                                  }
                                , Cmd.none
                                )

                            else
                                -- Check GitHub status and proceed accordingly
                                case state.githubStatus of
                                    GitHubConnected _ ->
                                        -- Already connected, fetch repos
                                        ( { model
                                            | page =
                                                Dashboard
                                                    { state
                                                        | view = CreateAppView { createState | step = SelectRepo [] "", error = Nothing }
                                                    }
                                          }
                                        , fetchRepos state.token
                                        )

                                    GitHubNotConnected ->
                                        -- Show GitHub connect option
                                        ( { model
                                            | page =
                                                Dashboard
                                                    { state
                                                        | view =
                                                            CreateAppView
                                                                { createState
                                                                    | step =
                                                                        ConnectGitHub
                                                                            { userCode = ""
                                                                            , verificationUri = ""
                                                                            , deviceCode = ""
                                                                            , expiresIn = 0
                                                                            , interval = 5
                                                                            , polling = False
                                                                            }
                                                                    , error = Nothing
                                                                }
                                                    }
                                          }
                                        , Cmd.none
                                        )

                                    GitHubUnknown ->
                                        -- Still loading, check again
                                        ( { model
                                            | page =
                                                Dashboard
                                                    { state
                                                        | view = CreateAppView { createState | step = CheckingGitHub, error = Nothing }
                                                    }
                                          }
                                        , fetchGitHubStatus state.token
                                        )

                        _ ->
                            ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        StartGitHubConnect ->
            case model.page of
                Dashboard state ->
                    ( model, startDeviceFlow state.token )

                _ ->
                    ( model, Cmd.none )

        GotDeviceFlowStart result ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        CreateAppView createState ->
                            case result of
                                Ok response ->
                                    ( { model
                                        | page =
                                            Dashboard
                                                { state
                                                    | view =
                                                        CreateAppView
                                                            { createState
                                                                | step =
                                                                    ConnectGitHub
                                                                        { userCode = response.userCode
                                                                        , verificationUri = response.verificationUri
                                                                        , deviceCode = response.deviceCode
                                                                        , expiresIn = response.expiresIn
                                                                        , interval = response.interval
                                                                        , polling = True
                                                                        }
                                                                , error = Nothing
                                                            }
                                                }
                                      }
                                    , startGitHubSSE
                                        { token = state.token
                                        , deviceCode = response.deviceCode
                                        , interval = response.interval
                                        , expiresIn = response.expiresIn
                                        }
                                    )

                                Err err ->
                                    ( { model
                                        | page =
                                            Dashboard
                                                { state
                                                    | view =
                                                        CreateAppView
                                                            { createState | error = Just (httpErrorToString err) }
                                                }
                                      }
                                    , Cmd.none
                                    )

                        _ ->
                            ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        GotGitHubSSEEvent value ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        CreateAppView createState ->
                            case Decode.decodeValue sseEventDecoder value of
                                Ok event ->
                                    case event.eventType of
                                        "success" ->
                                            -- Parse the data as JSON to get username
                                            case Decode.decodeString (Decode.field "username" Decode.string) event.data of
                                                Ok username ->
                                                    ( { model
                                                        | page =
                                                            Dashboard
                                                                { state
                                                                    | githubStatus = GitHubConnected username
                                                                    , view =
                                                                        CreateAppView
                                                                            { createState
                                                                                | step = SelectRepo [] ""
                                                                                , error = Nothing
                                                                            }
                                                                }
                                                      }
                                                    , fetchRepos state.token
                                                    )

                                                Err _ ->
                                                    -- Fallback: still a success, just no username parsed
                                                    ( { model
                                                        | page =
                                                            Dashboard
                                                                { state
                                                                    | githubStatus = GitHubConnected ""
                                                                    , view =
                                                                        CreateAppView
                                                                            { createState
                                                                                | step = SelectRepo [] ""
                                                                                , error = Nothing
                                                                            }
                                                                }
                                                      }
                                                    , fetchRepos state.token
                                                    )

                                        "error" ->
                                            -- Stop polling and show error
                                            case createState.step of
                                                ConnectGitHub ghState ->
                                                    ( { model
                                                        | page =
                                                            Dashboard
                                                                { state
                                                                    | view =
                                                                        CreateAppView
                                                                            { createState
                                                                                | step = ConnectGitHub { ghState | polling = False }
                                                                                , error = Just event.data
                                                                            }
                                                                }
                                                      }
                                                    , Cmd.none
                                                    )

                                                _ ->
                                                    ( model, Cmd.none )

                                        "pending" ->
                                            -- Still waiting, keep polling (subscription handles this)
                                            ( model, Cmd.none )

                                        _ ->
                                            -- Unknown event type, ignore
                                            ( model, Cmd.none )

                                Err _ ->
                                    -- Failed to decode event, ignore
                                    ( model, Cmd.none )

                        _ ->
                            ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        GotRepoList result ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        CreateAppView createState ->
                            case result of
                                Ok repos ->
                                    case createState.step of
                                        SelectRepo _ query ->
                                            ( { model
                                                | page =
                                                    Dashboard
                                                        { state
                                                            | view =
                                                                CreateAppView
                                                                    { createState | step = SelectRepo repos query }
                                                        }
                                              }
                                            , Cmd.none
                                            )

                                        _ ->
                                            ( { model
                                                | page =
                                                    Dashboard
                                                        { state
                                                            | view =
                                                                CreateAppView
                                                                    { createState | step = SelectRepo repos "" }
                                                        }
                                              }
                                            , Cmd.none
                                            )

                                Err err ->
                                    ( { model
                                        | page =
                                            Dashboard
                                                { state
                                                    | view =
                                                        CreateAppView
                                                            { createState | error = Just (httpErrorToString err) }
                                                }
                                      }
                                    , Cmd.none
                                    )

                        _ ->
                            ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        RepoSearchChanged query ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        CreateAppView createState ->
                            case createState.step of
                                SelectRepo repos _ ->
                                    ( { model
                                        | page =
                                            Dashboard
                                                { state
                                                    | view =
                                                        CreateAppView
                                                            { createState | step = SelectRepo repos query }
                                                }
                                      }
                                    , Cmd.none
                                    )

                                _ ->
                                    ( model, Cmd.none )

                        _ ->
                            ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        ChooseRepo repo ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        CreateAppView createState ->
                            ( { model
                                | page =
                                    Dashboard
                                        { state
                                            | view =
                                                CreateAppView
                                                    { createState | step = Creating, error = Nothing }
                                        }
                              }
                            , createAppWithRepo state.token createState.appName repo.fullName
                            )

                        _ ->
                            ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        SkipRepoSelection ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        CreateAppView createState ->
                            ( { model
                                | page =
                                    Dashboard
                                        { state
                                            | view =
                                                CreateAppView
                                                    { createState | step = Creating, error = Nothing }
                                        }
                              }
                            , createApp state.token createState.appName
                            )

                        _ ->
                            ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        GotAppCreated result ->
            case model.page of
                Dashboard state ->
                    case result of
                        Ok app ->
                            -- App created, add to list and go back to apps view
                            ( { model
                                | page =
                                    Dashboard
                                        { state
                                            | apps = app :: state.apps
                                            , view = AppsListView
                                        }
                              }
                            , Cmd.none
                            )

                        Err err ->
                            case state.view of
                                CreateAppView createState ->
                                    ( { model
                                        | page =
                                            Dashboard
                                                { state
                                                    | view =
                                                        CreateAppView
                                                            { createState
                                                                | step = EnterName
                                                                , error = Just (httpErrorToString err)
                                                            }
                                                }
                                      }
                                    , Cmd.none
                                    )

                                _ ->
                                    ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        -- App detail messages
        ViewAppDetail appName ->
            case model.page of
                Dashboard state ->
                    ( model
                    , Cmd.batch
                        [ Nav.pushUrl model.navKey ("/apps/" ++ appName)
                        , fetchAppDetail state.token appName
                        ]
                    )

                _ ->
                    ( model, Cmd.none )

        GotAppDetail result ->
            case model.page of
                Dashboard state ->
                    case result of
                        Ok app ->
                            let
                                detailState =
                                    { app = app
                                    , logs = ""
                                    , logsLoading = True
                                    , actionInProgress = Nothing
                                    , error = Nothing
                                    }
                            in
                            ( { model | page = Dashboard { state | view = AppDetailView detailState } }
                            , fetchAppLogs state.token app.name
                            )

                        Err err ->
                            ( { model
                                | page =
                                    Dashboard
                                        { state
                                            | view = AppsListView
                                        }
                              }
                            , Cmd.none
                            )

                _ ->
                    ( model, Cmd.none )

        BackToApps ->
            case model.page of
                Dashboard state ->
                    ( { model | page = Dashboard { state | view = AppsListView } }
                    , Cmd.batch
                        [ Nav.pushUrl model.navKey "/dashboard"
                        , fetchApps state.token
                        ]
                    )

                _ ->
                    ( model, Cmd.none )

        RefreshAppDetail ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        AppDetailView detailState ->
                            ( model
                            , Cmd.batch
                                [ fetchAppDetail state.token detailState.app.name
                                , fetchAppLogs state.token detailState.app.name
                                ]
                            )

                        _ ->
                            ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        StartApp ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        AppDetailView detailState ->
                            ( { model
                                | page =
                                    Dashboard
                                        { state
                                            | view =
                                                AppDetailView
                                                    { detailState
                                                        | actionInProgress = Just Starting
                                                        , error = Nothing
                                                    }
                                        }
                              }
                            , startAppRequest state.token detailState.app.name
                            )

                        _ ->
                            ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        StopApp ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        AppDetailView detailState ->
                            ( { model
                                | page =
                                    Dashboard
                                        { state
                                            | view =
                                                AppDetailView
                                                    { detailState
                                                        | actionInProgress = Just Stopping
                                                        , error = Nothing
                                                    }
                                        }
                              }
                            , stopAppRequest state.token detailState.app.name
                            )

                        _ ->
                            ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        BuildApp ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        AppDetailView detailState ->
                            ( { model
                                | page =
                                    Dashboard
                                        { state
                                            | view =
                                                AppDetailView
                                                    { detailState
                                                        | actionInProgress = Just Building
                                                        , error = Nothing
                                                    }
                                        }
                              }
                            , buildAppRequest state.token detailState.app.name
                            )

                        _ ->
                            ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        DeleteApp ->
            -- This just triggers confirmation, actual delete is ConfirmDeleteApp
            ( model, Cmd.none )

        ConfirmDeleteApp ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        AppDetailView detailState ->
                            ( { model
                                | page =
                                    Dashboard
                                        { state
                                            | view =
                                                AppDetailView
                                                    { detailState
                                                        | actionInProgress = Just Deleting
                                                        , error = Nothing
                                                    }
                                        }
                              }
                            , deleteAppRequest state.token detailState.app.name
                            )

                        _ ->
                            ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        CancelDeleteApp ->
            ( model, Cmd.none )

        GotAppStarted result ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        AppDetailView detailState ->
                            case result of
                                Ok _ ->
                                    ( { model
                                        | page =
                                            Dashboard
                                                { state
                                                    | view =
                                                        AppDetailView
                                                            { detailState | actionInProgress = Nothing }
                                                }
                                      }
                                    , fetchAppDetail state.token detailState.app.name
                                    )

                                Err err ->
                                    ( { model
                                        | page =
                                            Dashboard
                                                { state
                                                    | view =
                                                        AppDetailView
                                                            { detailState
                                                                | actionInProgress = Nothing
                                                                , error = Just (httpErrorToString err)
                                                            }
                                                }
                                      }
                                    , Cmd.none
                                    )

                        _ ->
                            ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        GotAppStopped result ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        AppDetailView detailState ->
                            case result of
                                Ok _ ->
                                    ( { model
                                        | page =
                                            Dashboard
                                                { state
                                                    | view =
                                                        AppDetailView
                                                            { detailState | actionInProgress = Nothing }
                                                }
                                      }
                                    , fetchAppDetail state.token detailState.app.name
                                    )

                                Err err ->
                                    ( { model
                                        | page =
                                            Dashboard
                                                { state
                                                    | view =
                                                        AppDetailView
                                                            { detailState
                                                                | actionInProgress = Nothing
                                                                , error = Just (httpErrorToString err)
                                                            }
                                                }
                                      }
                                    , Cmd.none
                                    )

                        _ ->
                            ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        GotAppBuilt result ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        AppDetailView detailState ->
                            case result of
                                Ok _ ->
                                    ( { model
                                        | page =
                                            Dashboard
                                                { state
                                                    | view =
                                                        AppDetailView
                                                            { detailState | actionInProgress = Nothing }
                                                }
                                      }
                                    , Cmd.batch
                                        [ fetchAppDetail state.token detailState.app.name
                                        , fetchAppLogs state.token detailState.app.name
                                        ]
                                    )

                                Err err ->
                                    ( { model
                                        | page =
                                            Dashboard
                                                { state
                                                    | view =
                                                        AppDetailView
                                                            { detailState
                                                                | actionInProgress = Nothing
                                                                , error = Just (httpErrorToString err)
                                                            }
                                                }
                                      }
                                    , Cmd.none
                                    )

                        _ ->
                            ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        GotAppDeleted result ->
            case model.page of
                Dashboard state ->
                    case result of
                        Ok _ ->
                            -- Remove app from list and go back to apps list
                            case state.view of
                                AppDetailView detailState ->
                                    let
                                        updatedApps =
                                            List.filter (\a -> a.name /= detailState.app.name) state.apps
                                    in
                                    ( { model
                                        | page =
                                            Dashboard
                                                { state
                                                    | view = AppsListView
                                                    , apps = updatedApps
                                                }
                                      }
                                    , Nav.pushUrl model.navKey "/dashboard"
                                    )

                                _ ->
                                    ( { model | page = Dashboard { state | view = AppsListView } }
                                    , Nav.pushUrl model.navKey "/dashboard"
                                    )

                        Err err ->
                            case state.view of
                                AppDetailView detailState ->
                                    ( { model
                                        | page =
                                            Dashboard
                                                { state
                                                    | view =
                                                        AppDetailView
                                                            { detailState
                                                                | actionInProgress = Nothing
                                                                , error = Just (httpErrorToString err)
                                                            }
                                                }
                                      }
                                    , Cmd.none
                                    )

                                _ ->
                                    ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        FetchLogs ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        AppDetailView detailState ->
                            ( { model
                                | page =
                                    Dashboard
                                        { state
                                            | view =
                                                AppDetailView
                                                    { detailState | logsLoading = True }
                                        }
                              }
                            , fetchAppLogs state.token detailState.app.name
                            )

                        _ ->
                            ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        GotLogs result ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        AppDetailView detailState ->
                            case result of
                                Ok logs ->
                                    ( { model
                                        | page =
                                            Dashboard
                                                { state
                                                    | view =
                                                        AppDetailView
                                                            { detailState
                                                                | logs = logs
                                                                , logsLoading = False
                                                            }
                                                }
                                      }
                                    , Cmd.none
                                    )

                                Err _ ->
                                    ( { model
                                        | page =
                                            Dashboard
                                                { state
                                                    | view =
                                                        AppDetailView
                                                            { detailState
                                                                | logs = ""
                                                                , logsLoading = False
                                                            }
                                                }
                                      }
                                    , Cmd.none
                                    )

                        _ ->
                            ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )



-- HTTP


checkServerStatus : Cmd Msg
checkServerStatus =
    Http.get
        { url = "/api/auth/status"
        , expect = Http.expectJson GotServerStatus serverStatusDecoder
        }


verifyToken : String -> Cmd Msg
verifyToken token =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/auth/me"
        , body = Http.emptyBody
        , expect = Http.expectJson GotTokenVerification (tokenVerificationDecoder token)
        , timeout = Nothing
        , tracker = Nothing
        }


submitLogin : LoginForm -> Cmd Msg
submitLogin form =
    Http.post
        { url = "/api/auth/login"
        , body =
            Http.jsonBody
                (Encode.object
                    [ ( "email", Encode.string form.email )
                    , ( "password", Encode.string form.password )
                    ]
                )
        , expect = Http.expectJson GotLoginResponse authResponseDecoder
        }


submitRegister : SetupForm -> Cmd Msg
submitRegister form =
    Http.post
        { url = "/api/auth/register"
        , body =
            Http.jsonBody
                (Encode.object
                    [ ( "email", Encode.string form.email )
                    , ( "password", Encode.string form.password )
                    , ( "full_name", Encode.string form.fullName )
                    , ( "organization_name", Encode.string form.organizationName )
                    ]
                )
        , expect = Http.expectJson GotRegisterResponse authResponseDecoder
        }


fetchApps : String -> Cmd Msg
fetchApps token =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps"
        , body = Http.emptyBody
        , expect = Http.expectJson GotApps appsListDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


fetchGitHubStatus : String -> Cmd Msg
fetchGitHubStatus token =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/github/status"
        , body = Http.emptyBody
        , expect = Http.expectJson GotGitHubStatus githubStatusDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


startDeviceFlow : String -> Cmd Msg
startDeviceFlow token =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/github/connect/start"
        , body = Http.emptyBody
        , expect = Http.expectJson GotDeviceFlowStart deviceFlowStartDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


fetchRepos : String -> Cmd Msg
fetchRepos token =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/github/repos"
        , body = Http.emptyBody
        , expect = Http.expectJson GotRepoList reposListDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


createApp : String -> String -> Cmd Msg
createApp token name =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps"
        , body =
            Http.jsonBody
                (Encode.object
                    [ ( "name", Encode.string name )
                    ]
                )
        , expect = Http.expectJson GotAppCreated appInfoDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


createAppWithRepo : String -> String -> String -> Cmd Msg
createAppWithRepo token name repoFullName =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps"
        , body =
            Http.jsonBody
                (Encode.object
                    [ ( "name", Encode.string name )
                    , ( "from_github", Encode.string repoFullName )
                    ]
                )
        , expect = Http.expectJson GotAppCreated appInfoDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


fetchAppDetail : String -> String -> Cmd Msg
fetchAppDetail token appName =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName
        , body = Http.emptyBody
        , expect = Http.expectJson GotAppDetail appDetailDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


startAppRequest : String -> String -> Cmd Msg
startAppRequest token appName =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/start"
        , body = Http.emptyBody
        , expect = Http.expectString GotAppStarted
        , timeout = Nothing
        , tracker = Nothing
        }


stopAppRequest : String -> String -> Cmd Msg
stopAppRequest token appName =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/stop"
        , body = Http.emptyBody
        , expect = Http.expectString GotAppStopped
        , timeout = Nothing
        , tracker = Nothing
        }


buildAppRequest : String -> String -> Cmd Msg
buildAppRequest token appName =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/build"
        , body = Http.emptyBody
        , expect = Http.expectString GotAppBuilt
        , timeout = Just 300000
        , tracker = Nothing
        }


deleteAppRequest : String -> String -> Cmd Msg
deleteAppRequest token appName =
    Http.request
        { method = "DELETE"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName
        , body = Http.emptyBody
        , expect = Http.expectString GotAppDeleted
        , timeout = Nothing
        , tracker = Nothing
        }


fetchAppLogs : String -> String -> Cmd Msg
fetchAppLogs token appName =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/logs?lines=100"
        , body = Http.emptyBody
        , expect = Http.expectString GotLogs
        , timeout = Nothing
        , tracker = Nothing
        }



-- DECODERS


serverStatusDecoder : Decode.Decoder ServerStatus
serverStatusDecoder =
    Decode.map2 ServerStatus
        (Decode.field "initialized" Decode.bool)
        (Decode.field "version" Decode.string)


userInfoDecoder : Decode.Decoder UserInfo
userInfoDecoder =
    Decode.map2 UserInfo
        (Decode.at [ "user", "email" ] Decode.string)
        (Decode.at [ "user", "full_name" ] Decode.string)


tokenVerificationDecoder : String -> Decode.Decoder TokenVerificationResponse
tokenVerificationDecoder token =
    Decode.map2 TokenVerificationResponse
        (Decode.map2 UserInfo
            (Decode.at [ "user", "email" ] Decode.string)
            (Decode.at [ "user", "full_name" ] Decode.string)
        )
        (Decode.succeed token)


authResponseDecoder : Decode.Decoder AuthResponse
authResponseDecoder =
    Decode.map2 AuthResponse
        (Decode.at [ "tokens", "access_token" ] Decode.string)
        (Decode.field "user" userDecoder)


userDecoder : Decode.Decoder UserInfo
userDecoder =
    Decode.map2 UserInfo
        (Decode.field "email" Decode.string)
        (Decode.field "full_name" Decode.string)


appsListDecoder : Decode.Decoder (List AppInfo)
appsListDecoder =
    Decode.list appInfoDecoder


appInfoDecoder : Decode.Decoder AppInfo
appInfoDecoder =
    Decode.map3 AppInfo
        (Decode.field "id" Decode.string)
        (Decode.field "name" Decode.string)
        (Decode.field "state" Decode.string)


githubStatusDecoder : Decode.Decoder GitHubStatusResponse
githubStatusDecoder =
    Decode.map2 GitHubStatusResponse
        (Decode.field "connected" Decode.bool)
        (Decode.maybe (Decode.field "username" Decode.string))


deviceFlowStartDecoder : Decode.Decoder DeviceFlowStartResponse
deviceFlowStartDecoder =
    Decode.map5 DeviceFlowStartResponse
        (Decode.field "user_code" Decode.string)
        (Decode.field "verification_uri" Decode.string)
        (Decode.field "device_code" Decode.string)
        (Decode.field "expires_in" Decode.int)
        (Decode.field "interval" Decode.int)


type alias SSEEvent =
    { eventType : String
    , data : String
    }


sseEventDecoder : Decode.Decoder SSEEvent
sseEventDecoder =
    Decode.map2 SSEEvent
        (Decode.field "type" Decode.string)
        (Decode.field "data" Decode.string)


reposListDecoder : Decode.Decoder (List RepoInfo)
reposListDecoder =
    Decode.list repoInfoDecoder


repoInfoDecoder : Decode.Decoder RepoInfo
repoInfoDecoder =
    Decode.map6 RepoInfo
        (Decode.field "name" Decode.string)
        (Decode.field "full_name" Decode.string)
        (Decode.maybe (Decode.field "description" Decode.string))
        (Decode.field "private" Decode.bool)
        (Decode.field "clone_url" Decode.string)
        (Decode.field "default_branch" Decode.string)


appDetailDecoder : Decode.Decoder AppDetail
appDetailDecoder =
    Decode.map7 AppDetail
        (Decode.field "id" Decode.string)
        (Decode.field "name" Decode.string)
        (Decode.field "state" Decode.string)
        (Decode.maybe (Decode.field "port" Decode.int))
        (Decode.field "created_at" Decode.string)
        (Decode.field "updated_at" Decode.string)
        (Decode.maybe (Decode.field "remote" remoteInfoDecoder))


remoteInfoDecoder : Decode.Decoder RemoteInfo
remoteInfoDecoder =
    Decode.map3 RemoteInfo
        (Decode.field "name" Decode.string)
        (Decode.field "url" Decode.string)
        (Decode.field "branch" Decode.string)



-- HELPERS


httpErrorToString : Http.Error -> String
httpErrorToString error =
    case error of
        Http.BadUrl url ->
            "Invalid URL: " ++ url

        Http.Timeout ->
            "Request timed out"

        Http.NetworkError ->
            "Network error - please check your connection"

        Http.BadStatus status ->
            case status of
                401 ->
                    "Invalid email or password"

                409 ->
                    "An account with this email already exists"

                _ ->
                    "Server error (status " ++ String.fromInt status ++ ")"

        Http.BadBody message ->
            "Error parsing response: " ++ message



-- VIEW


view : Model -> Browser.Document Msg
view model =
    { title = getPageTitle model.page
    , body =
        [ case model.page of
            Loading ->
                viewLoading

            Setup form ->
                viewSetup form model.serverVersion

            Login form ->
                viewLogin form model.serverVersion

            Dashboard state ->
                viewDashboard state model.serverVersion
        ]
    }


getPageTitle : Page -> String
getPageTitle page =
    case page of
        Loading ->
            "Loading - Litehouse"

        Setup _ ->
            "Setup - Litehouse"

        Login _ ->
            "Login - Litehouse"

        Dashboard state ->
            case state.view of
                AppsListView ->
                    "Dashboard - Litehouse"

                CreateAppView _ ->
                    "Create App - Litehouse"

                AppDetailView detailState ->
                    detailState.app.name ++ " - Litehouse"


viewLoading : Html Msg
viewLoading =
    div [ class "min-h-screen bg-litehouse-bg flex items-center justify-center" ]
        [ div [ class "bg-litehouse-surface rounded-2xl shadow-soft p-10 text-center" ]
            [ div [ class "w-10 h-10 border-3 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow mx-auto mb-4" ] []
            , p [ class "text-litehouse-muted" ] [ text "Loading..." ]
            ]
        ]


viewSetup : SetupForm -> String -> Html Msg
viewSetup form version =
    div [ class "min-h-screen bg-litehouse-bg flex items-center justify-center p-5" ]
        [ div [ class "bg-litehouse-surface rounded-2xl shadow-soft p-10 w-full max-w-md text-center" ]
            [ h1 [ class "text-2xl font-semibold text-litehouse-text mb-2" ] [ text "Welcome to Litehouse" ]
            , p [ class "text-litehouse-muted mb-6" ] [ text "Create your admin account to get started" ]
            , Html.form [ onSubmit SubmitSetup ]
                [ viewError form.error
                , div [ class "mb-4 text-left" ]
                    [ label [ for "fullName", class "block mb-1 text-sm font-medium text-litehouse-text" ] [ text "Full Name" ]
                    , input
                        [ type_ "text"
                        , id "fullName"
                        , value form.fullName
                        , onInput SetupFullNameChanged
                        , placeholder "John Doe"
                        , required True
                        , disabled form.submitting
                        , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50 disabled:bg-litehouse-bg disabled:cursor-not-allowed"
                        ]
                        []
                    ]
                , div [ class "mb-4 text-left" ]
                    [ label [ for "email", class "block mb-1 text-sm font-medium text-litehouse-text" ] [ text "Email" ]
                    , input
                        [ type_ "email"
                        , id "email"
                        , value form.email
                        , onInput SetupEmailChanged
                        , placeholder "admin@example.com"
                        , required True
                        , disabled form.submitting
                        , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50 disabled:bg-litehouse-bg disabled:cursor-not-allowed"
                        ]
                        []
                    ]
                , div [ class "mb-4 text-left" ]
                    [ label [ for "orgName", class "block mb-1 text-sm font-medium text-litehouse-text" ] [ text "Organization Name" ]
                    , input
                        [ type_ "text"
                        , id "orgName"
                        , value form.organizationName
                        , onInput SetupOrganizationNameChanged
                        , placeholder "My Organization"
                        , required True
                        , disabled form.submitting
                        , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50 disabled:bg-litehouse-bg disabled:cursor-not-allowed"
                        ]
                        []
                    ]
                , div [ class "mb-4 text-left" ]
                    [ label [ for "password", class "block mb-1 text-sm font-medium text-litehouse-text" ] [ text "Password" ]
                    , input
                        [ type_ "password"
                        , id "password"
                        , value form.password
                        , onInput SetupPasswordChanged
                        , placeholder "At least 8 characters"
                        , required True
                        , disabled form.submitting
                        , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50 disabled:bg-litehouse-bg disabled:cursor-not-allowed"
                        ]
                        []
                    ]
                , div [ class "mb-4 text-left" ]
                    [ label [ for "confirmPassword", class "block mb-1 text-sm font-medium text-litehouse-text" ] [ text "Confirm Password" ]
                    , input
                        [ type_ "password"
                        , id "confirmPassword"
                        , value form.confirmPassword
                        , onInput SetupConfirmPasswordChanged
                        , placeholder "Re-enter your password"
                        , required True
                        , disabled form.submitting
                        , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50 disabled:bg-litehouse-bg disabled:cursor-not-allowed"
                        ]
                        []
                    ]
                , button
                    [ type_ "submit"
                    , class "w-full px-4 py-2.5 bg-litehouse-amber hover:bg-litehouse-amberDeep text-white font-medium rounded-xl transition-colors mt-2 disabled:bg-litehouse-border disabled:cursor-not-allowed"
                    , disabled form.submitting
                    ]
                    [ text
                        (if form.submitting then
                            "Creating Account..."

                         else
                            "Create Account"
                        )
                    ]
                ]
            , viewVersion version
            ]
        ]


viewLogin : LoginForm -> String -> Html Msg
viewLogin form version =
    div [ class "min-h-screen bg-litehouse-bg flex items-center justify-center p-5" ]
        [ div [ class "bg-litehouse-surface rounded-2xl shadow-soft p-10 w-full max-w-md text-center" ]
            [ h1 [ class "text-2xl font-semibold text-litehouse-text mb-2" ] [ text "Litehouse" ]
            , p [ class "text-litehouse-muted mb-6" ] [ text "Sign in to your account" ]
            , Html.form [ onSubmit SubmitLogin ]
                [ viewError form.error
                , div [ class "mb-4 text-left" ]
                    [ label [ for "email", class "block mb-1 text-sm font-medium text-litehouse-text" ] [ text "Email" ]
                    , input
                        [ type_ "email"
                        , id "email"
                        , value form.email
                        , onInput LoginEmailChanged
                        , placeholder "admin@example.com"
                        , required True
                        , disabled form.submitting
                        , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50 disabled:bg-litehouse-bg disabled:cursor-not-allowed"
                        ]
                        []
                    ]
                , div [ class "mb-4 text-left" ]
                    [ label [ for "password", class "block mb-1 text-sm font-medium text-litehouse-text" ] [ text "Password" ]
                    , input
                        [ type_ "password"
                        , id "password"
                        , value form.password
                        , onInput LoginPasswordChanged
                        , placeholder "Your password"
                        , required True
                        , disabled form.submitting
                        , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50 disabled:bg-litehouse-bg disabled:cursor-not-allowed"
                        ]
                        []
                    ]
                , button
                    [ type_ "submit"
                    , class "w-full px-4 py-2.5 bg-litehouse-amber hover:bg-litehouse-amberDeep text-white font-medium rounded-xl transition-colors mt-2 disabled:bg-litehouse-border disabled:cursor-not-allowed"
                    , disabled form.submitting
                    ]
                    [ text
                        (if form.submitting then
                            "Signing in..."

                         else
                            "Sign In"
                        )
                    ]
                ]
            , viewVersion version
            ]
        ]


viewDashboard : DashboardState -> String -> Html Msg
viewDashboard state version =
    div [ class "min-h-screen bg-litehouse-bg flex flex-col" ]
        [ viewHeader state
        , div [ class "flex flex-1" ]
            [ viewSidebar state
            , main_ [ class "flex-1 p-6" ]
                [ div [ class "max-w-6xl mx-auto" ]
                    [ case state.view of
                        AppsListView ->
                            viewAppsList state

                        CreateAppView createState ->
                            viewCreateApp state createState

                        AppDetailView detailState ->
                            viewAppDetail detailState
                    ]
                ]
            ]
        , footer [ class "p-4 text-center border-t border-litehouse-border" ]
            [ viewVersion version
            ]
        ]


viewHeader : DashboardState -> Html Msg
viewHeader state =
    header [ class "bg-litehouse-surface border-b border-litehouse-border px-6 py-4 flex justify-between items-center" ]
        [ div [ class "flex items-center gap-4" ]
            [ a [ href "/dashboard", class "text-xl font-semibold text-litehouse-text hover:opacity-80 transition-opacity" ] [ text "Litehouse" ]
            ]
        , div [ class "flex items-center gap-4" ]
            [ viewGitHubStatusBadge state.githubStatus
            , span [ class "text-sm text-litehouse-muted" ] [ text state.user.fullName ]
            , button
                [ class "px-4 py-2.5 bg-litehouse-amber hover:bg-litehouse-amberDeep text-white font-medium rounded-xl transition-colors"
                , onClick ShowCreateApp
                ]
                [ text "+ New App" ]
            , button
                [ class "px-4 py-2 border border-litehouse-border text-litehouse-muted hover:bg-litehouse-bg rounded-xl transition-colors"
                , onClick Logout
                ]
                [ text "Logout" ]
            ]
        ]


viewSidebar : DashboardState -> Html Msg
viewSidebar state =
    aside [ class "w-56 bg-litehouse-surface border-r border-litehouse-border p-4" ]
        [ nav [ class "space-y-1" ]
            [ viewSidebarItem "My Apps" MyApps state.activeSidebarItem
            , viewSidebarItem "Activity" Activity state.activeSidebarItem
            , viewSidebarItem "Backups" Backups state.activeSidebarItem
            , viewSidebarItem "Settings" Settings state.activeSidebarItem
            ]
        ]


viewSidebarItem : String -> SidebarItem -> SidebarItem -> Html Msg
viewSidebarItem label item activeItem =
    let
        isActive =
            item == activeItem

        baseClasses =
            "block w-full px-3 py-2 rounded-xl text-sm font-medium transition-colors text-left"

        activeClasses =
            if isActive then
                "bg-litehouse-amber/10 text-litehouse-amber"

            else
                "text-litehouse-muted hover:bg-litehouse-bg hover:text-litehouse-text"
    in
    button [ class (baseClasses ++ " " ++ activeClasses) ] [ text label ]


viewGitHubStatusBadge : GitHubStatus -> Html Msg
viewGitHubStatusBadge status =
    case status of
        GitHubConnected username ->
            span [ class "px-2.5 py-1 rounded-full text-xs font-medium bg-litehouse-success/20 text-litehouse-success" ]
                [ text ("GitHub: " ++ username) ]

        GitHubNotConnected ->
            span [ class "px-2.5 py-1 rounded-full text-xs font-medium bg-litehouse-warning/20 text-litehouse-warning" ]
                [ text "GitHub: Not connected" ]

        GitHubUnknown ->
            text ""


viewAppsList : DashboardState -> Html Msg
viewAppsList state =
    div []
        [ div [ class "flex justify-between items-center mb-6" ]
            [ h2 [ class "text-xl font-semibold text-litehouse-text" ] [ text "My Apps" ]
            ]
        , if state.appsLoading then
            div [ class "flex flex-col items-center justify-center py-16 text-litehouse-muted" ]
                [ div [ class "w-10 h-10 border-3 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow mb-4" ] []
                , p [] [ text "Loading apps..." ]
                ]

          else if List.isEmpty state.apps then
            div [ class "bg-litehouse-surface rounded-2xl shadow-soft p-12 text-center" ]
                [ p [ class "text-litehouse-muted mb-4" ] [ text "No apps yet. Create your first app to get started." ]
                , button
                    [ class "px-4 py-2.5 bg-litehouse-amber hover:bg-litehouse-amberDeep text-white font-medium rounded-xl transition-colors"
                    , onClick ShowCreateApp
                    ]
                    [ text "Create App" ]
                ]

          else
            div [ class "grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4" ]
                (List.map viewAppCard state.apps)
        ]


viewAppCard : AppInfo -> Html Msg
viewAppCard app =
    a
        [ class "block bg-litehouse-surface rounded-2xl shadow-soft border border-litehouse-border p-5 hover:border-litehouse-amber transition-colors cursor-pointer"
        , href ("/apps/" ++ app.name)
        ]
        [ div [ class "flex justify-between items-start mb-3" ]
            [ h3 [ class "text-base font-semibold text-litehouse-text" ] [ text app.name ]
            , viewStatusBadge app.state
            ]
        , div [ class "text-xs text-litehouse-muted font-mono" ]
            [ text ("ID: " ++ app.id) ]
        ]


viewStatusBadge : String -> Html Msg
viewStatusBadge state =
    let
        ( bgColor, textColor ) =
            case state of
                "running" ->
                    ( "bg-litehouse-success/20", "text-litehouse-success" )

                "starting" ->
                    ( "bg-litehouse-warning/20", "text-litehouse-warning" )

                "building" ->
                    ( "bg-litehouse-warning/20", "text-litehouse-warning" )

                "stopped" ->
                    ( "bg-litehouse-error/20", "text-litehouse-error" )

                "error" ->
                    ( "bg-litehouse-error/20", "text-litehouse-error" )

                _ ->
                    ( "bg-litehouse-border/50", "text-litehouse-muted" )
    in
    span [ class ("px-2.5 py-1 rounded-full text-xs font-medium uppercase " ++ bgColor ++ " " ++ textColor) ]
        [ text state ]


viewAppDetail : AppDetailState -> Html Msg
viewAppDetail state =
    let
        app =
            state.app

        isRunning =
            app.state == "running" || app.state == "starting"

        hasRemote =
            state.app.remote /= Nothing

        actionDisabled =
            state.actionInProgress /= Nothing
    in
    div [ class "space-y-6" ]
        [ -- Header with back button
          div [ class "flex items-center justify-between" ]
            [ div [ class "flex items-center gap-4" ]
                [ button
                    [ class "text-litehouse-slateBlue hover:bg-litehouse-mistBlue px-3 py-1.5 rounded-xl transition-colors text-sm"
                    , onClick BackToApps
                    ]
                    [ text "< Apps" ]
                , h2 [ class "text-2xl font-semibold text-litehouse-text" ] [ text app.name ]
                , viewStatusBadge app.state
                ]
            , button
                [ class "px-4 py-2 border border-litehouse-border text-litehouse-muted hover:bg-litehouse-bg rounded-xl transition-colors disabled:opacity-50"
                , onClick RefreshAppDetail
                , disabled actionDisabled
                ]
                [ text "Refresh" ]
            ]

        -- Error message
        , viewError state.error

        -- Action in progress indicator
        , case state.actionInProgress of
            Just action ->
                div [ class "flex items-center gap-3 p-4 bg-litehouse-warning/10 text-litehouse-warning rounded-xl" ]
                    [ div [ class "w-5 h-5 border-2 border-litehouse-warning/30 border-t-litehouse-warning rounded-full animate-spin-slow" ] []
                    , span []
                        [ text
                            (case action of
                                Starting ->
                                    "Starting app..."

                                Stopping ->
                                    "Stopping app..."

                                Building ->
                                    "Building app (this may take a while)..."

                                Deleting ->
                                    "Deleting app..."
                            )
                        ]
                    ]

            Nothing ->
                text ""

        -- Info section
        , div [ class "bg-litehouse-surface rounded-2xl shadow-soft border border-litehouse-border p-6" ]
            [ h3 [ class "text-base font-semibold text-litehouse-text mb-4" ] [ text "Information" ]
            , div [ class "grid grid-cols-2 md:grid-cols-4 gap-4" ]
                [ div []
                    [ span [ class "block text-xs text-litehouse-muted uppercase font-medium mb-1" ] [ text "ID" ]
                    , span [ class "text-sm text-litehouse-text font-mono break-all" ] [ text app.id ]
                    ]
                , div []
                    [ span [ class "block text-xs text-litehouse-muted uppercase font-medium mb-1" ] [ text "State" ]
                    , viewStatusBadge app.state
                    ]
                , div []
                    [ span [ class "block text-xs text-litehouse-muted uppercase font-medium mb-1" ] [ text "Port" ]
                    , span [ class "text-sm text-litehouse-text" ]
                        [ text
                            (case app.port_ of
                                Just p ->
                                    String.fromInt p

                                Nothing ->
                                    "Not assigned"
                            )
                        ]
                    ]
                , div []
                    [ span [ class "block text-xs text-litehouse-muted uppercase font-medium mb-1" ] [ text "Created" ]
                    , span [ class "text-sm text-litehouse-text" ] [ text app.createdAt ]
                    ]
                ]
            ]

        -- Repository section
        , div [ class "bg-litehouse-surface rounded-2xl shadow-soft border border-litehouse-border p-6" ]
            [ h3 [ class "text-base font-semibold text-litehouse-text mb-4" ] [ text "Repository" ]
            , case app.remote of
                Just remote ->
                    div [ class "grid grid-cols-2 gap-4" ]
                        [ div []
                            [ span [ class "block text-xs text-litehouse-muted uppercase font-medium mb-1" ] [ text "URL" ]
                            , span [ class "text-sm text-litehouse-text font-mono break-all" ] [ text remote.url ]
                            ]
                        , div []
                            [ span [ class "block text-xs text-litehouse-muted uppercase font-medium mb-1" ] [ text "Branch" ]
                            , span [ class "text-sm text-litehouse-text" ] [ text remote.branch ]
                            ]
                        ]

                Nothing ->
                    p [ class "text-sm text-litehouse-muted italic" ] [ text "No repository connected" ]
            ]

        -- Actions section
        , div [ class "bg-litehouse-surface rounded-2xl shadow-soft border border-litehouse-border p-6" ]
            [ h3 [ class "text-base font-semibold text-litehouse-text mb-4" ] [ text "Actions" ]
            , div [ class "flex flex-wrap gap-3" ]
                [ -- Start/Stop button
                  if isRunning then
                    button
                        [ class "px-5 py-2.5 bg-litehouse-warning hover:bg-litehouse-warning/80 text-white font-medium rounded-xl transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                        , onClick StopApp
                        , disabled actionDisabled
                        ]
                        [ text "Stop" ]

                  else
                    button
                        [ class "px-5 py-2.5 bg-litehouse-success hover:bg-litehouse-success/80 text-white font-medium rounded-xl transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                        , onClick StartApp
                        , disabled actionDisabled
                        ]
                        [ text "Start" ]

                -- Build button (only if has remote)
                , if hasRemote then
                    button
                        [ class "px-5 py-2.5 bg-litehouse-slateBlue hover:bg-litehouse-slateBlue/80 text-white font-medium rounded-xl transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                        , onClick BuildApp
                        , disabled actionDisabled
                        ]
                        [ text "Build" ]

                  else
                    text ""

                -- Delete button
                , button
                    [ class "px-5 py-2.5 bg-litehouse-error hover:bg-litehouse-error/80 text-white font-medium rounded-xl transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                    , onClick ConfirmDeleteApp
                    , disabled actionDisabled
                    ]
                    [ text "Delete" ]
                ]
            ]

        -- Logs section
        , div [ class "bg-litehouse-surface rounded-2xl shadow-soft border border-litehouse-border p-6" ]
            [ div [ class "flex justify-between items-center mb-4" ]
                [ h3 [ class "text-base font-semibold text-litehouse-text" ] [ text "Logs" ]
                , button
                    [ class "px-3 py-1.5 text-sm border border-litehouse-border text-litehouse-muted hover:bg-litehouse-bg rounded-xl transition-colors disabled:opacity-50"
                    , onClick FetchLogs
                    , disabled state.logsLoading
                    ]
                    [ text "Refresh Logs" ]
                ]
            , if state.logsLoading then
                div [ class "flex items-center justify-center gap-3 py-10 text-litehouse-muted" ]
                    [ div [ class "w-5 h-5 border-2 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow" ] []
                    , span [] [ text "Loading logs..." ]
                    ]

              else if String.isEmpty state.logs then
                div [ class "py-10 text-center text-litehouse-muted" ]
                    [ text "No logs available" ]

              else
                pre [ class "bg-gray-900 text-gray-300 font-mono text-xs p-4 rounded-xl overflow-auto max-h-96 whitespace-pre-wrap break-all" ]
                    [ text state.logs ]
            ]
        ]


viewCreateApp : DashboardState -> CreateAppState -> Html Msg
viewCreateApp dashState createState =
    div [ class "max-w-xl" ]
        [ div [ class "bg-litehouse-surface rounded-2xl shadow-soft border border-litehouse-border p-6" ]
            [ div [ class "flex justify-between items-center mb-6 pb-4 border-b border-litehouse-border" ]
                [ h2 [ class "text-xl font-semibold text-litehouse-text" ] [ text "Create New App" ]
                , button
                    [ class "px-4 py-2 border border-litehouse-border text-litehouse-muted hover:bg-litehouse-bg rounded-xl transition-colors"
                    , onClick CancelCreateApp
                    ]
                    [ text "Cancel" ]
                ]
            , viewError createState.error
            , case createState.step of
                EnterName ->
                    viewEnterName createState

                CheckingGitHub ->
                    viewCheckingGitHub

                ConnectGitHub ghState ->
                    viewConnectGitHub ghState

                SelectRepo repos query ->
                    viewSelectRepo repos query

                Creating ->
                    viewCreating
            ]
        ]


viewEnterName : CreateAppState -> Html Msg
viewEnterName createState =
    div [ class "py-4" ]
        [ h3 [ class "text-base font-medium text-litehouse-text mb-4" ] [ text "Step 1: Name your app" ]
        , Html.form [ onSubmit SubmitAppName ]
            [ div [ class "mb-4" ]
                [ label [ for "appName", class "block mb-1 text-sm font-medium text-litehouse-text" ] [ text "App Name" ]
                , input
                    [ type_ "text"
                    , id "appName"
                    , value createState.appName
                    , onInput AppNameChanged
                    , placeholder "my-awesome-app"
                    , required True
                    , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50"
                    ]
                    []
                , p [ class "mt-1 text-xs text-litehouse-muted" ] [ text "Use lowercase letters, numbers, and hyphens only" ]
                ]
            , button
                [ type_ "submit"
                , class "px-4 py-2.5 bg-litehouse-amber hover:bg-litehouse-amberDeep text-white font-medium rounded-xl transition-colors"
                ]
                [ text "Next" ]
            ]
        ]


viewCheckingGitHub : Html Msg
viewCheckingGitHub =
    div [ class "py-8 text-center" ]
        [ div [ class "w-10 h-10 border-3 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow mx-auto mb-4" ] []
        , p [ class "text-litehouse-muted" ] [ text "Checking GitHub connection..." ]
        ]


viewConnectGitHub : GitHubConnectState -> Html Msg
viewConnectGitHub ghState =
    div [ class "py-4" ]
        [ h3 [ class "text-base font-medium text-litehouse-text mb-4" ] [ text "Step 2: Connect GitHub (Optional)" ]
        , if String.isEmpty ghState.userCode then
            div [ class "text-center py-4" ]
                [ p [ class "text-litehouse-muted mb-4" ] [ text "Connect your GitHub account to import repositories." ]
                , div [ class "flex justify-center gap-3" ]
                    [ button
                        [ class "px-4 py-2.5 bg-litehouse-amber hover:bg-litehouse-amberDeep text-white font-medium rounded-xl transition-colors"
                        , onClick StartGitHubConnect
                        ]
                        [ text "Connect GitHub" ]
                    , button
                        [ class "px-4 py-2 border border-litehouse-border text-litehouse-muted hover:bg-litehouse-bg rounded-xl transition-colors"
                        , onClick SkipRepoSelection
                        ]
                        [ text "Skip" ]
                    ]
                ]

          else
            div [ class "text-center py-4" ]
                [ p [ class "text-litehouse-muted mb-2" ] [ text "Go to GitHub and enter this code:" ]
                , div [ class "text-3xl font-bold font-mono tracking-widest py-5 px-6 bg-litehouse-bg rounded-xl my-4 text-litehouse-text" ]
                    [ text ghState.userCode ]
                , p [ class "text-litehouse-muted" ]
                    [ text "Open "
                    , a [ href ghState.verificationUri, target "_blank", class "text-litehouse-slateBlue hover:underline" ] [ text ghState.verificationUri ]
                    , text " and enter the code above."
                    ]
                , if ghState.polling then
                    div [ class "flex items-center justify-center gap-3 mt-4 text-litehouse-muted" ]
                        [ div [ class "w-5 h-5 border-2 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow" ] []
                        , span [] [ text "Waiting for authorization..." ]
                        ]

                  else
                    button
                        [ class "mt-4 px-4 py-2 border border-litehouse-border text-litehouse-muted hover:bg-litehouse-bg rounded-xl transition-colors"
                        , onClick SkipRepoSelection
                        ]
                        [ text "Skip" ]
                ]
        ]


viewSelectRepo : List RepoInfo -> String -> Html Msg
viewSelectRepo repos query =
    let
        filteredRepos =
            if String.isEmpty query then
                repos

            else
                List.filter
                    (\repo ->
                        String.contains (String.toLower query) (String.toLower repo.name)
                            || String.contains (String.toLower query) (String.toLower repo.fullName)
                    )
                    repos
    in
    div [ class "py-4" ]
        [ h3 [ class "text-base font-medium text-litehouse-text mb-4" ] [ text "Step 3: Select Repository (Optional)" ]
        , div [ class "mb-4" ]
            [ input
                [ type_ "text"
                , placeholder "Search repositories..."
                , value query
                , onInput RepoSearchChanged
                , class "w-full px-3 py-2.5 border border-litehouse-border rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber/50"
                ]
                []
            ]
        , if List.isEmpty repos then
            div [ class "py-10 text-center" ]
                [ div [ class "w-10 h-10 border-3 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow mx-auto mb-4" ] []
                , p [ class "text-litehouse-muted" ] [ text "Loading repositories..." ]
                ]

          else
            div []
                [ div [ class "max-h-80 overflow-y-auto border border-litehouse-border rounded-xl" ]
                    (List.map viewRepoItem filteredRepos)
                , div [ class "mt-4 text-center" ]
                    [ button
                        [ class "px-4 py-2 border border-litehouse-border text-litehouse-muted hover:bg-litehouse-bg rounded-xl transition-colors"
                        , onClick SkipRepoSelection
                        ]
                        [ text "Create without repository" ]
                    ]
                ]
        ]


viewRepoItem : RepoInfo -> Html Msg
viewRepoItem repo =
    div
        [ class "px-4 py-3 border-b border-litehouse-border last:border-b-0 cursor-pointer hover:bg-litehouse-bg transition-colors"
        , onClick (ChooseRepo repo)
        ]
        [ div [ class "flex justify-between items-center mb-1" ]
            [ span [ class "font-medium text-litehouse-text" ] [ text repo.fullName ]
            , if repo.private then
                span [ class "px-2 py-0.5 rounded text-xs font-medium bg-litehouse-warning/20 text-litehouse-warning" ] [ text "Private" ]

              else
                span [ class "px-2 py-0.5 rounded text-xs font-medium bg-litehouse-border/50 text-litehouse-muted" ] [ text "Public" ]
            ]
        , case repo.description of
            Just desc ->
                p [ class "text-sm text-litehouse-muted truncate mb-1" ] [ text desc ]

            Nothing ->
                text ""
        , span [ class "text-xs text-litehouse-muted" ] [ text ("Default branch: " ++ repo.defaultBranch) ]
        ]


viewCreating : Html Msg
viewCreating =
    div [ class "py-8 text-center" ]
        [ div [ class "w-10 h-10 border-3 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow mx-auto mb-4" ] []
        , p [ class "text-litehouse-muted" ] [ text "Creating app..." ]
        ]


viewError : Maybe String -> Html Msg
viewError maybeError =
    case maybeError of
        Just error ->
            div [ class "bg-litehouse-error/10 text-litehouse-error p-3 rounded-xl mb-4 text-sm text-left" ] [ text error ]

        Nothing ->
            text ""


viewVersion : String -> Html Msg
viewVersion version =
    if String.isEmpty version then
        text ""

    else
        p [ class "text-xs text-litehouse-muted" ] [ text ("v" ++ version) ]
