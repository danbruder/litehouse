port module Main exposing (main)

import Browser
import Html exposing (..)
import Html.Attributes exposing (..)
import Html.Events exposing (onClick, onInput, onSubmit)
import Http
import Json.Decode as Decode
import Json.Encode as Encode
import Time


-- PORTS


port saveToken : String -> Cmd msg


port clearToken : () -> Cmd msg



-- MAIN


main : Program Flags Model Msg
main =
    Browser.element
        { init = init
        , update = update
        , view = view
        , subscriptions = subscriptions
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


type alias DashboardState =
    { user : UserInfo
    , view : DashboardView
    , apps : List AppInfo
    , appsLoading : Bool
    , githubStatus : GitHubStatus
    , token : String
    }


type DashboardView
    = AppsListView
    | CreateAppView CreateAppState


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


init : Flags -> ( Model, Cmd Msg )
init flags =
    case flags.token of
        Just token ->
            -- We have a token, verify it
            ( { page = Loading, serverVersion = "" }
            , verifyToken token
            )

        Nothing ->
            -- No token, check server status
            ( { page = Loading, serverVersion = "" }
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
                                Time.every (toFloat ghState.interval * 1000) (\_ -> PollGitHubConnect)

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
    = GotServerStatus (Result Http.Error ServerStatus)
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
    | PollGitHubConnect
    | GotGitHubConnectPoll (Result Http.Error GitHubConnectResponse)
    | GotRepoList (Result Http.Error (List RepoInfo))
    | RepoSearchChanged String
    | ChooseRepo RepoInfo
    | SkipRepoSelection
    | GotAppCreated (Result Http.Error AppInfo)


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


type alias GitHubConnectResponse =
    { username : String
    }


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case msg of
        GotServerStatus result ->
            case result of
                Ok status ->
                    if status.initialized then
                        ( { model
                            | page = Login emptyLoginForm
                            , serverVersion = status.version
                          }
                        , Cmd.none
                        )

                    else
                        ( { model
                            | page = Setup emptySetupForm
                            , serverVersion = status.version
                          }
                        , Cmd.none
                        )

                Err _ ->
                    ( { model | page = Login { emptyLoginForm | error = Just "Failed to connect to server" } }
                    , Cmd.none
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
                            }
                    in
                    ( { model | page = Dashboard dashboardState }
                    , Cmd.batch
                        [ fetchApps response.token
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
                                    }
                            in
                            ( { model | page = Dashboard dashboardState }
                            , Cmd.batch
                                [ saveToken response.accessToken
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
                                    }
                            in
                            ( { model | page = Dashboard dashboardState }
                            , Cmd.batch
                                [ saveToken response.accessToken
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
            , clearToken ()
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

        PollGitHubConnect ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        CreateAppView createState ->
                            case createState.step of
                                ConnectGitHub ghState ->
                                    ( model, pollDeviceFlow state.token ghState.deviceCode ghState.interval ghState.expiresIn )

                                _ ->
                                    ( model, Cmd.none )

                        _ ->
                            ( model, Cmd.none )

                _ ->
                    ( model, Cmd.none )

        GotGitHubConnectPoll result ->
            case model.page of
                Dashboard state ->
                    case state.view of
                        CreateAppView createState ->
                            case result of
                                Ok response ->
                                    -- Successfully connected, update status and fetch repos
                                    ( { model
                                        | page =
                                            Dashboard
                                                { state
                                                    | githubStatus = GitHubConnected response.username
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

                                Err err ->
                                    -- Check if it's a "pending" error (authorization_pending)
                                    -- In that case, keep polling. Otherwise, show error
                                    case err of
                                        Http.BadStatus 202 ->
                                            -- Still pending, keep polling
                                            ( model, Cmd.none )

                                        _ ->
                                            -- Real error, stop polling
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


pollDeviceFlow : String -> String -> Int -> Int -> Cmd Msg
pollDeviceFlow token deviceCode interval expiresIn =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/github/connect/poll"
        , body =
            Http.jsonBody
                (Encode.object
                    [ ( "device_code", Encode.string deviceCode )
                    , ( "interval", Encode.int interval )
                    , ( "expires_in", Encode.int expiresIn )
                    ]
                )
        , expect = Http.expectJson GotGitHubConnectPoll githubConnectResponseDecoder
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


githubConnectResponseDecoder : Decode.Decoder GitHubConnectResponse
githubConnectResponseDecoder =
    Decode.map GitHubConnectResponse
        (Decode.field "username" Decode.string)


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


view : Model -> Html Msg
view model =
    case model.page of
        Loading ->
            viewLoading

        Setup form ->
            viewSetup form model.serverVersion

        Login form ->
            viewLogin form model.serverVersion

        Dashboard state ->
            viewDashboard state model.serverVersion


viewLoading : Html Msg
viewLoading =
    div [ class "auth-container" ]
        [ div [ class "auth-card" ]
            [ div [ class "spinner" ] []
            , p [ class "loading-text" ] [ text "Loading..." ]
            ]
        ]


viewSetup : SetupForm -> String -> Html Msg
viewSetup form version =
    div [ class "auth-container" ]
        [ div [ class "auth-card" ]
            [ h1 [] [ text "Welcome to Litehouse" ]
            , p [ class "subtitle" ] [ text "Create your admin account to get started" ]
            , Html.form [ onSubmit SubmitSetup ]
                [ viewError form.error
                , div [ class "form-group" ]
                    [ label [ for "fullName" ] [ text "Full Name" ]
                    , input
                        [ type_ "text"
                        , id "fullName"
                        , value form.fullName
                        , onInput SetupFullNameChanged
                        , placeholder "John Doe"
                        , required True
                        , disabled form.submitting
                        ]
                        []
                    ]
                , div [ class "form-group" ]
                    [ label [ for "email" ] [ text "Email" ]
                    , input
                        [ type_ "email"
                        , id "email"
                        , value form.email
                        , onInput SetupEmailChanged
                        , placeholder "admin@example.com"
                        , required True
                        , disabled form.submitting
                        ]
                        []
                    ]
                , div [ class "form-group" ]
                    [ label [ for "orgName" ] [ text "Organization Name" ]
                    , input
                        [ type_ "text"
                        , id "orgName"
                        , value form.organizationName
                        , onInput SetupOrganizationNameChanged
                        , placeholder "My Organization"
                        , required True
                        , disabled form.submitting
                        ]
                        []
                    ]
                , div [ class "form-group" ]
                    [ label [ for "password" ] [ text "Password" ]
                    , input
                        [ type_ "password"
                        , id "password"
                        , value form.password
                        , onInput SetupPasswordChanged
                        , placeholder "At least 8 characters"
                        , required True
                        , disabled form.submitting
                        ]
                        []
                    ]
                , div [ class "form-group" ]
                    [ label [ for "confirmPassword" ] [ text "Confirm Password" ]
                    , input
                        [ type_ "password"
                        , id "confirmPassword"
                        , value form.confirmPassword
                        , onInput SetupConfirmPasswordChanged
                        , placeholder "Re-enter your password"
                        , required True
                        , disabled form.submitting
                        ]
                        []
                    ]
                , button
                    [ type_ "submit"
                    , class "btn-primary"
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
    div [ class "auth-container" ]
        [ div [ class "auth-card" ]
            [ h1 [] [ text "Litehouse" ]
            , p [ class "subtitle" ] [ text "Sign in to your account" ]
            , Html.form [ onSubmit SubmitLogin ]
                [ viewError form.error
                , div [ class "form-group" ]
                    [ label [ for "email" ] [ text "Email" ]
                    , input
                        [ type_ "email"
                        , id "email"
                        , value form.email
                        , onInput LoginEmailChanged
                        , placeholder "admin@example.com"
                        , required True
                        , disabled form.submitting
                        ]
                        []
                    ]
                , div [ class "form-group" ]
                    [ label [ for "password" ] [ text "Password" ]
                    , input
                        [ type_ "password"
                        , id "password"
                        , value form.password
                        , onInput LoginPasswordChanged
                        , placeholder "Your password"
                        , required True
                        , disabled form.submitting
                        ]
                        []
                    ]
                , button
                    [ type_ "submit"
                    , class "btn-primary"
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
    div [ class "dashboard" ]
        [ header [ class "dashboard-header" ]
            [ div [ class "header-brand" ]
                [ h1 [] [ text "Litehouse" ]
                ]
            , div [ class "header-user" ]
                [ viewGitHubStatusBadge state.githubStatus
                , span [ class "user-name" ] [ text state.user.fullName ]
                , button [ class "btn-secondary", onClick Logout ] [ text "Logout" ]
                ]
            ]
        , main_ [ class "dashboard-content" ]
            [ case state.view of
                AppsListView ->
                    viewAppsList state

                CreateAppView createState ->
                    viewCreateApp state createState
            ]
        , footer [ class "dashboard-footer" ]
            [ viewVersion version
            ]
        ]


viewGitHubStatusBadge : GitHubStatus -> Html Msg
viewGitHubStatusBadge status =
    case status of
        GitHubConnected username ->
            span [ class "github-badge github-connected" ]
                [ text ("GitHub: " ++ username) ]

        GitHubNotConnected ->
            span [ class "github-badge github-not-connected" ]
                [ text "GitHub: Not connected" ]

        GitHubUnknown ->
            text ""


viewAppsList : DashboardState -> Html Msg
viewAppsList state =
    div [ class "apps-section" ]
        [ div [ class "apps-header" ]
            [ h2 [] [ text "Apps" ]
            , button [ class "btn-primary", onClick ShowCreateApp ] [ text "Create App" ]
            ]
        , if state.appsLoading then
            div [ class "apps-loading" ]
                [ div [ class "spinner" ] []
                , p [] [ text "Loading apps..." ]
                ]

          else if List.isEmpty state.apps then
            div [ class "apps-empty" ]
                [ p [] [ text "No apps yet. Create your first app to get started." ]
                ]

          else
            div [ class "apps-list" ]
                (List.map viewAppCard state.apps)
        ]


viewAppCard : AppInfo -> Html Msg
viewAppCard app =
    div [ class "app-card" ]
        [ div [ class "app-card-header" ]
            [ h3 [ class "app-name" ] [ text app.name ]
            , span [ class ("app-state app-state-" ++ app.state) ] [ text app.state ]
            ]
        , div [ class "app-card-footer" ]
            [ span [ class "app-id" ] [ text ("ID: " ++ app.id) ]
            ]
        ]


viewCreateApp : DashboardState -> CreateAppState -> Html Msg
viewCreateApp dashState createState =
    div [ class "create-app-wizard" ]
        [ div [ class "wizard-header" ]
            [ h2 [] [ text "Create New App" ]
            , button [ class "btn-secondary", onClick CancelCreateApp ] [ text "Cancel" ]
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


viewEnterName : CreateAppState -> Html Msg
viewEnterName createState =
    div [ class "wizard-step" ]
        [ h3 [] [ text "Step 1: Name your app" ]
        , Html.form [ onSubmit SubmitAppName ]
            [ div [ class "form-group" ]
                [ label [ for "appName" ] [ text "App Name" ]
                , input
                    [ type_ "text"
                    , id "appName"
                    , value createState.appName
                    , onInput AppNameChanged
                    , placeholder "my-awesome-app"
                    , required True
                    ]
                    []
                , p [ class "form-help" ] [ text "Use lowercase letters, numbers, and hyphens only" ]
                ]
            , button [ type_ "submit", class "btn-primary" ] [ text "Next" ]
            ]
        ]


viewCheckingGitHub : Html Msg
viewCheckingGitHub =
    div [ class "wizard-step" ]
        [ div [ class "spinner" ] []
        , p [] [ text "Checking GitHub connection..." ]
        ]


viewConnectGitHub : GitHubConnectState -> Html Msg
viewConnectGitHub ghState =
    div [ class "wizard-step" ]
        [ h3 [] [ text "Step 2: Connect GitHub (Optional)" ]
        , if String.isEmpty ghState.userCode then
            div [ class "github-connect-prompt" ]
                [ p [] [ text "Connect your GitHub account to import repositories." ]
                , div [ class "button-group" ]
                    [ button [ class "btn-primary", onClick StartGitHubConnect ] [ text "Connect GitHub" ]
                    , button [ class "btn-secondary", onClick SkipRepoSelection ] [ text "Skip" ]
                    ]
                ]

          else
            div [ class "github-connect-code" ]
                [ p [] [ text "Go to GitHub and enter this code:" ]
                , div [ class "user-code" ] [ text ghState.userCode ]
                , p []
                    [ text "Open "
                    , a [ href ghState.verificationUri, target "_blank" ] [ text ghState.verificationUri ]
                    , text " and enter the code above."
                    ]
                , if ghState.polling then
                    div [ class "polling-indicator" ]
                        [ div [ class "spinner spinner-small" ] []
                        , span [] [ text "Waiting for authorization..." ]
                        ]

                  else
                    button [ class "btn-secondary", onClick SkipRepoSelection ] [ text "Skip" ]
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
    div [ class "wizard-step" ]
        [ h3 [] [ text "Step 3: Select Repository (Optional)" ]
        , div [ class "form-group" ]
            [ input
                [ type_ "text"
                , placeholder "Search repositories..."
                , value query
                , onInput RepoSearchChanged
                , class "search-input"
                ]
                []
            ]
        , if List.isEmpty repos then
            div [ class "repos-loading" ]
                [ div [ class "spinner" ] []
                , p [] [ text "Loading repositories..." ]
                ]

          else
            div []
                [ div [ class "repo-list" ]
                    (List.map viewRepoItem filteredRepos)
                , div [ class "skip-section" ]
                    [ button [ class "btn-secondary", onClick SkipRepoSelection ]
                        [ text "Create without repository" ]
                    ]
                ]
        ]


viewRepoItem : RepoInfo -> Html Msg
viewRepoItem repo =
    div [ class "repo-item", onClick (ChooseRepo repo) ]
        [ div [ class "repo-item-header" ]
            [ span [ class "repo-name" ] [ text repo.fullName ]
            , if repo.private then
                span [ class "repo-badge repo-private" ] [ text "Private" ]

              else
                span [ class "repo-badge repo-public" ] [ text "Public" ]
            ]
        , case repo.description of
            Just desc ->
                p [ class "repo-description" ] [ text desc ]

            Nothing ->
                text ""
        , span [ class "repo-branch" ] [ text ("Default branch: " ++ repo.defaultBranch) ]
        ]


viewCreating : Html Msg
viewCreating =
    div [ class "wizard-step" ]
        [ div [ class "spinner" ] []
        , p [] [ text "Creating app..." ]
        ]


viewError : Maybe String -> Html Msg
viewError maybeError =
    case maybeError of
        Just error ->
            div [ class "error-message" ] [ text error ]

        Nothing ->
            text ""


viewVersion : String -> Html Msg
viewVersion version =
    if String.isEmpty version then
        text ""

    else
        p [ class "version" ] [ text ("v" ++ version) ]
