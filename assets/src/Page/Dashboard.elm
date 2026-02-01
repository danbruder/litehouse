module Page.Dashboard exposing
    ( Model
    , Msg(..)
    , DashboardView(..)
    , CreateAppState
    , CreateAppStep
    , GitHubConnectState
    , SidebarItem(..)
    , init
    , update
    , view
    , save
    , load
    -- Exposed for testing
    , handleGotGitHubStatus
    , handleBuildStatusEvent
    , handleBuildLogsEvent
    , handleContainerLogsEvent
    , AppAction(..)
    , LogsView(..)
    )

import Effect exposing (Effect)
import Html exposing (Html, a, button, div, h2, h3, input, label, option, p, pre, select, span, text)
import Html.Attributes exposing (class, disabled, for, href, id, placeholder, required, selected, target, title, type_, value)
import Html.Events exposing (onClick, onInput, onSubmit)
import Json.Decode as Decode
import Json.Encode as Encode
import Page.Dashboard.CreateApp as CreateApp
import Page.Dashboard.Data as Data
import Page.Dashboard.EnvVars as EnvVars
import Page.Dashboard.Settings as Settings
import Shared


-- MODEL


type alias Model =
    { view : DashboardView
    , apps : List Effect.AppInfo
    , appsLoading : Bool
    , activeSidebarItem : SidebarItem
    }


type DashboardView
    = AppsListView
    | CreateAppView CreateAppState
    | AppDetailView AppDetailState
    | SettingsView SettingsState


-- Type aliases to maintain backward compatibility
type alias CreateAppState =
    CreateApp.State


type alias CreateAppStep =
    CreateApp.Step


type alias GitHubConnectState =
    CreateApp.GitHubConnectState


type alias AppDetailState =
    { app : Effect.AppDetail
    , logs : String
    , logsLoading : Bool
    , logsView : LogsView
    , builds : List Effect.BuildInfo
    , selectedBuildId : Maybe String
    , buildLogs : String
    , buildLogsLoading : Bool
    , actionInProgress : Maybe AppAction
    , error : Maybe String
    , streamingBuildId : Maybe String
    , buildLogsStreaming : Bool
    , envVarsModel : EnvVars.Model
    }


type alias SettingsState =
    Settings.Model


type AppAction
    = Starting
    | Stopping
    | Building
    | Deleting


type LogsView
    = RuntimeLogs
    | BuildLogs


type SidebarItem
    = MyApps
    | Activity
    | Backups
    | Settings


emptyCreateAppState : CreateAppState
emptyCreateAppState =
    CreateApp.init


-- INIT


init : Shared.Model navigationKey -> ( Model, Effect Msg )
init shared =
    let
        effects =
            case ( shared.token, shared.user ) of
                ( Just token, Just _ ) ->
                    Effect.batch
                        [ Effect.FetchApps token GotApps
                        , Effect.FetchGitHubStatus token GotGitHubStatus
                        ]

                _ ->
                    Effect.none
    in
    ( { view = AppsListView
      , apps = []
      , appsLoading = True
      , activeSidebarItem = MyApps
      }
    , effects
    )


-- UPDATE


type Msg
    = GotApps (Result String (List Effect.AppInfo))
    | GotGitHubStatus (Result String Effect.GitHubStatusResponse)
      -- Create app flow
    | ShowCreateApp
    | CancelCreateApp
    | AppNameChanged String
    | SubmitAppName
    | StartGitHubConnect
    | GotDeviceFlowStart (Result String Effect.DeviceFlowStartResponse)
    | GotRepoList (Result String (List Effect.RepoInfo))
    | RepoSearchChanged String
    | ChooseRepo Effect.RepoInfo
    | SkipRepoSelection
    | GotAppCreated (Result String Effect.AppInfo)
      -- App detail
    | ViewAppDetail String
    | GotAppDetail (Result String Effect.AppDetail)
    | BackToApps
    | RefreshAppDetail
    | StartApp
    | StopApp
    | BuildApp
    | DeleteApp
    | ConfirmDeleteApp
    | CancelDeleteApp
    | GotAppStarted (Result String String)
    | GotAppStopped (Result String String)
    | GotAppBuilt (Result String String)
    | GotAppDeleted (Result String String)
    | NoOp
    | FetchLogs
    | GotLogs (Result String String)
      -- Build logs
    | SwitchLogsView LogsView
    | GotBuilds (Result String (List Effect.BuildInfo))
    | SelectBuild String
    | GotBuildLogs (Result String String)
      -- Environment variables
    | EnvVarsMsg EnvVars.Msg
      -- Settings
    | ShowSettings
    | SettingsMsg Settings.Msg
      -- Unified SSE message handler
    | HandleSSEEvent Decode.Value


update : Shared.Model navigationKey -> Msg -> Model -> ( Model, Effect Msg )
update shared msg model =
    case msg of
        GotApps result ->
            case result of
                Ok apps ->
                    ( { model | apps = apps, appsLoading = False }
                    , Effect.none
                    )

                Err _ ->
                    ( { model | appsLoading = False }
                    , Effect.none
                    )

        GotGitHubStatus result ->
            case model.view of
                CreateAppView createState ->
                    let
                        ( newStep, newError, effect ) =
                            handleGotGitHubStatus result createState shared.token
                    in
                    ( { model
                        | view = CreateAppView { createState | step = newStep, error = newError }
                      }
                    , effect
                    )

                _ ->
                    ( model, Effect.none )

        ShowCreateApp ->
            let
                ( newCreateState, createAppEffect ) =
                    CreateApp.update shared CreateApp.ShowCreateApp emptyCreateAppState
            in
            ( { model | view = CreateAppView newCreateState }
            , Effect.map mapCreateAppMsg createAppEffect
            )

        CancelCreateApp ->
            case model.view of
                CreateAppView createState ->
                    let
                        ( newCreateState, createAppEffect ) =
                            CreateApp.update shared CreateApp.CancelCreateApp createState
                    in
                    ( { model | view = AppsListView }
                    , Effect.map mapCreateAppMsg createAppEffect
                    )

                _ ->
                    ( model, Effect.none )

        AppNameChanged name ->
            case model.view of
                CreateAppView createState ->
                    let
                        ( newCreateState, createAppEffect ) =
                            CreateApp.update shared (CreateApp.AppNameChanged name) createState
                    in
                    ( { model | view = CreateAppView newCreateState }
                    , Effect.map mapCreateAppMsg createAppEffect
                    )

                _ ->
                    ( model, Effect.none )

        SubmitAppName ->
            case model.view of
                CreateAppView createState ->
                    let
                        ( newCreateState, createAppEffect ) =
                            CreateApp.update shared CreateApp.SubmitAppName createState
                    in
                    -- Apply the effect from Dashboard-level logic
                    let
                        effect =
                            if String.isEmpty (String.trim newCreateState.appName) then
                                Effect.none

                            else
                                case shared.githubStatus of
                                    Shared.GitHubConnected _ ->
                                        case shared.token of
                                            Just token ->
                                                Effect.FetchRepos token GotRepoList

                                            Nothing ->
                                                Effect.none

                                    Shared.GitHubNotConnected ->
                                        Effect.none

                                    Shared.GitHubUnknown ->
                                        case shared.token of
                                            Just token ->
                                                Effect.FetchGitHubStatus token GotGitHubStatus

                                            Nothing ->
                                                Effect.none
                    in
                    ( { model | view = CreateAppView newCreateState }
                    , effect
                    )

                _ ->
                    ( model, Effect.none )

        StartGitHubConnect ->
            case model.view of
                CreateAppView createState ->
                    let
                        ( newCreateState, createAppEffect ) =
                            CreateApp.update shared CreateApp.StartGitHubConnect createState
                    in
                    ( { model | view = CreateAppView newCreateState }
                    , Effect.map mapCreateAppMsg createAppEffect
                    )

                _ ->
                    ( model, Effect.none )

        GotDeviceFlowStart result ->
            case model.view of
                CreateAppView createState ->
                    let
                        ( newCreateState, createAppEffect ) =
                            CreateApp.update shared (CreateApp.GotDeviceFlowStart result) createState
                    in
                    case shared.token of
                        Just token ->
                            let
                                effect =
                                    case result of
                                        Ok response ->
                                            Effect.StartGitHubPolling
                                                token
                                                response.deviceCode
                                                response.interval
                                                response.expiresIn

                                        Err _ ->
                                            Effect.none
                            in
                            ( { model | view = CreateAppView newCreateState }
                            , effect
                            )

                        Nothing ->
                            ( { model | view = CreateAppView newCreateState }
                            , Effect.none
                            )

                _ ->
                    ( model, Effect.none )

        GotRepoList result ->
            case model.view of
                CreateAppView createState ->
                    let
                        ( newCreateState, createAppEffect ) =
                            CreateApp.update shared (CreateApp.GotRepoList result) createState
                    in
                    ( { model | view = CreateAppView newCreateState }
                    , Effect.map mapCreateAppMsg createAppEffect
                    )

                _ ->
                    ( model, Effect.none )

        RepoSearchChanged query ->
            case model.view of
                CreateAppView createState ->
                    let
                        ( newCreateState, createAppEffect ) =
                            CreateApp.update shared (CreateApp.RepoSearchChanged query) createState
                    in
                    ( { model | view = CreateAppView newCreateState }
                    , Effect.map mapCreateAppMsg createAppEffect
                    )

                _ ->
                    ( model, Effect.none )

        ChooseRepo repo ->
            case ( model.view, shared.token ) of
                ( CreateAppView createState, Just token ) ->
                    let
                        ( newCreateState, createAppEffect ) =
                            CreateApp.update shared (CreateApp.ChooseRepo repo) createState
                    in
                    ( { model
                        | view = CreateAppView newCreateState
                      }
                    , Effect.CreateAppWithRepo token createState.appName repo.fullName GotAppCreated
                    )

                _ ->
                    ( model, Effect.none )

        SkipRepoSelection ->
            case ( model.view, shared.token ) of
                ( CreateAppView createState, Just token ) ->
                    let
                        ( newCreateState, createAppEffect ) =
                            CreateApp.update shared CreateApp.SkipRepoSelection createState
                    in
                    ( { model
                        | view = CreateAppView newCreateState
                      }
                    , Effect.CreateApp token createState.appName GotAppCreated
                    )

                _ ->
                    ( model, Effect.none )

        GotAppCreated result ->
            case result of
                Ok app ->
                    -- App created, add to list and go back to apps view
                    ( { model
                        | apps = app :: model.apps
                        , appsLoading = False
                        , view = AppsListView
                      }
                    , Effect.none
                    )

                Err err ->
                    case model.view of
                        CreateAppView createState ->
                            ( { model
                                | view =
                                    CreateAppView
                                        { createState
                                            | step = CreateApp.EnterName
                                            , error = Just err
                                        }
                              }
                            , Effect.none
                            )

                        _ ->
                            ( model, Effect.none )

        ViewAppDetail appName ->
            case shared.token of
                Just token ->
                    ( model
                    , Effect.FetchAppDetail token appName GotAppDetail
                    )

                Nothing ->
                    ( model, Effect.none )

        GotAppDetail result ->
            case result of
                Ok app ->
                    -- Preserve streaming state if we're already on this app's detail view
                    let
                        ( streamingBuildId, buildLogsStreaming, buildLogs ) =
                            case model.view of
                                AppDetailView detailState ->
                                    if detailState.app.name == app.name then
                                        ( detailState.streamingBuildId, detailState.buildLogsStreaming, detailState.buildLogs )

                                    else
                                        ( Nothing, False, "" )

                                _ ->
                                    ( Nothing, False, "" )
                        
                        -- Start log streaming if app is running
                        logStreamingEffect =
                            if app.state == "running" then
                                case shared.token of
                                    Just token ->
                                        Effect.StartLogStreaming token app.name (\_ -> NoOp)
                                    
                                    Nothing ->
                                        Effect.none
                            else
                                Effect.none

                        -- Update SSE filters to only receive messages for this app
                        sseFilterEffect =
                            Effect.UpdateSSEFilters (Just (Data.encodeAppFilter app.name))
                        
                        -- Fetch env vars if we have a token
                        envVarsEffect =
                            case shared.token of
                                Just token ->
                                    Effect.FetchEnvVars token app.name (EnvVarsMsg << EnvVars.GotEnvVars)

                                Nothing ->
                                    Effect.none
                    in
                    ( { model
                        | view =
                            AppDetailView
                                { app = app
                                , logs = ""
                                , logsLoading = False
                                , logsView = RuntimeLogs
                                , builds = []
                                , selectedBuildId = Nothing
                                , buildLogs = buildLogs
                                , buildLogsLoading = False
                                , actionInProgress = Nothing
                                , error = Nothing
                                , streamingBuildId = streamingBuildId
                                , buildLogsStreaming = buildLogsStreaming
                                , envVarsModel = EnvVars.init
                                }
                      }
                    , Effect.batch
                        [ logStreamingEffect
                        , sseFilterEffect
                        , envVarsEffect
                        ]
                    )

                Err err ->
                    -- Failed to fetch app detail, stay on current view
                    ( model, Effect.none )

        BackToApps ->
            -- Stop any streaming and clear SSE filters when navigating away
            let
                clearFilterEffect =
                    Effect.UpdateSSEFilters Nothing
            in
            ( { model | view = AppsListView }
            , Effect.batch [ clearFilterEffect, Effect.PushUrl "/dashboard" ]
            )

        RefreshAppDetail ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    ( model
                    , Effect.FetchAppDetail token detailState.app.name GotAppDetail
                    )

                _ ->
                    ( model, Effect.none )

        StartApp ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    ( { model
                        | view =
                            AppDetailView
                                { detailState | actionInProgress = Just Starting, error = Nothing }
                      }
                    , Effect.StartApp token detailState.app.name GotAppStarted
                    )

                _ ->
                    ( model, Effect.none )

        StopApp ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    ( { model
                        | view =
                            AppDetailView
                                { detailState | actionInProgress = Just Stopping, error = Nothing }
                      }
                    , Effect.StopApp token detailState.app.name GotAppStopped
                    )

                _ ->
                    ( model, Effect.none )

        BuildApp ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    ( { model
                        | view =
                            AppDetailView
                                { detailState | actionInProgress = Just Building, error = Nothing }
                      }
                    , Effect.BuildApp token detailState.app.name GotAppBuilt
                    )

                _ ->
                    ( model, Effect.none )

        ConfirmDeleteApp ->
            -- In a real implementation, we'd show a confirmation dialog
            -- For now, just proceed with deletion
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    ( { model
                        | view =
                            AppDetailView
                                { detailState | actionInProgress = Just Deleting, error = Nothing }
                      }
                    , Effect.DeleteApp token detailState.app.name GotAppDeleted
                    )

                _ ->
                    ( model, Effect.none )

        CancelDeleteApp ->
            -- Not implemented yet
            ( model, Effect.none )

        DeleteApp ->
            -- Not implemented yet
            ( model, Effect.none )

        GotAppStarted result ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    case result of
                        Ok _ ->
                            ( { model
                                | view = AppDetailView { detailState | actionInProgress = Nothing }
                              }
                            , Effect.FetchAppDetail token detailState.app.name GotAppDetail
                            )

                        Err err ->
                            ( { model
                                | view =
                                    AppDetailView
                                        { detailState
                                            | actionInProgress = Nothing
                                            , error = Just err
                                        }
                              }
                            , Effect.none
                            )

                _ ->
                    ( model, Effect.none )

        GotAppStopped result ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    case result of
                        Ok _ ->
                            ( { model
                                | view = AppDetailView { detailState | actionInProgress = Nothing }
                              }
                            , Effect.FetchAppDetail token detailState.app.name GotAppDetail
                            )

                        Err err ->
                            ( { model
                                | view =
                                    AppDetailView
                                        { detailState
                                            | actionInProgress = Nothing
                                            , error = Just err
                                        }
                              }
                            , Effect.none
                            )

                _ ->
                    ( model, Effect.none )

        GotAppBuilt result ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    case result of
                        Ok responseJson ->
                            -- Parse build_id from JSON response: {"message": "...", "build_id": "..."}
                            case Decode.decodeString (Decode.field "build_id" Decode.string) responseJson of
                                Ok buildId ->
                                    -- Build logs will stream automatically via unified SSE
                                    ( { model
                                        | view =
                                            AppDetailView
                                                { detailState
                                                    | actionInProgress = Just Building
                                                    , logsView = BuildLogs
                                                    , buildLogs = ""
                                                    , streamingBuildId = Just buildId
                                                    , buildLogsStreaming = True
                                                    , selectedBuildId = Just buildId
                                                }
                                      }
                                    , Effect.none
                                    )

                                Err _ ->
                                    -- Couldn't parse build_id, fall back to old behavior
                                    ( { model
                                        | view = AppDetailView { detailState | actionInProgress = Nothing }
                                      }
                                    , Effect.batch
                                        [ Effect.FetchAppDetail token detailState.app.name GotAppDetail
                                        , Effect.FetchBuilds token detailState.app.name GotBuilds
                                        ]
                                    )

                        Err err ->
                            ( { model
                                | view =
                                    AppDetailView
                                        { detailState
                                            | actionInProgress = Nothing
                                            , error = Just err
                                        }
                              }
                            , Effect.FetchBuilds token detailState.app.name GotBuilds
                            )

                _ ->
                    ( model, Effect.none )

        GotAppDeleted result ->
            case result of
                Ok _ ->
                    -- App deleted, go back to apps list
                    ( { model | view = AppsListView }
                    , Effect.PushUrl "/dashboard"
                    )

                Err err ->
                    case model.view of
                        AppDetailView detailState ->
                            ( { model
                                | view =
                                    AppDetailView
                                        { detailState
                                            | actionInProgress = Nothing
                                            , error = Just err
                                        }
                              }
                            , Effect.none
                            )

                        _ ->
                            ( model, Effect.none )

        NoOp ->
            -- No operation, used for fire-and-forget effects
            ( model, Effect.none )

        FetchLogs ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    ( { model
                        | view = AppDetailView { detailState | logsLoading = True }
                      }
                    , Effect.FetchLogs token detailState.app.name GotLogs
                    )

                _ ->
                    ( model, Effect.none )

        GotLogs result ->
            case model.view of
                AppDetailView detailState ->
                    case result of
                        Ok logs ->
                            ( { model
                                | view =
                                    AppDetailView
                                        { detailState
                                            | logs = logs
                                            , logsLoading = False
                                        }
                              }
                            , Effect.none
                            )

                        Err _ ->
                            ( { model
                                | view = AppDetailView { detailState | logsLoading = False }
                              }
                            , Effect.none
                            )

                _ ->
                    ( model, Effect.none )

        SwitchLogsView logsView ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    let
                        updatedDetailState =
                            { detailState | logsView = logsView }

                        effect =
                            case logsView of
                                BuildLogs ->
                                    if List.isEmpty detailState.builds then
                                        Effect.FetchBuilds token detailState.app.name GotBuilds

                                    else
                                        Effect.none

                                RuntimeLogs ->
                                    if String.isEmpty detailState.logs then
                                        Effect.FetchLogs token detailState.app.name GotLogs

                                    else
                                        Effect.none
                    in
                    ( { model | view = AppDetailView updatedDetailState }
                    , effect
                    )

                _ ->
                    ( model, Effect.none )

        GotBuilds result ->
            case model.view of
                AppDetailView detailState ->
                    case result of
                        Ok builds ->
                            let
                                selectedBuildId =
                                    case builds of
                                        first :: _ ->
                                            Just first.id

                                        [] ->
                                            Nothing

                                effect =
                                    case ( shared.token, selectedBuildId ) of
                                        ( Just token, Just buildId ) ->
                                            Effect.FetchBuildLogs token detailState.app.name buildId GotBuildLogs

                                        _ ->
                                            Effect.none
                            in
                            ( { model
                                | view =
                                    AppDetailView
                                        { detailState
                                            | builds = builds
                                            , selectedBuildId = selectedBuildId
                                            , buildLogsLoading = selectedBuildId /= Nothing
                                        }
                              }
                            , effect
                            )

                        Err _ ->
                            ( model, Effect.none )

                _ ->
                    ( model, Effect.none )

        SelectBuild buildId ->
            case ( model.view, shared.token ) of
                ( AppDetailView detailState, Just token ) ->
                    ( { model
                        | view =
                            AppDetailView
                                { detailState
                                    | selectedBuildId = Just buildId
                                    , buildLogsLoading = True
                                }
                      }
                    , Effect.FetchBuildLogs token detailState.app.name buildId GotBuildLogs
                    )

                _ ->
                    ( model, Effect.none )

        GotBuildLogs result ->
            case model.view of
                AppDetailView detailState ->
                    case result of
                        Ok logs ->
                            ( { model
                                | view =
                                    AppDetailView
                                        { detailState
                                            | buildLogs = logs
                                            , buildLogsLoading = False
                                        }
                              }
                            , Effect.none
                            )

                        Err _ ->
                            ( { model
                                | view = AppDetailView { detailState | buildLogsLoading = False }
                              }
                            , Effect.none
                            )

                _ ->
                    ( model, Effect.none )

        EnvVarsMsg envVarsMsg ->
            case model.view of
                AppDetailView detailState ->
                    let
                        token =
                            Maybe.withDefault "" shared.token

                        ( updatedEnvVarsModel, envVarsEffect, maybeError ) =
                            EnvVars.update envVarsMsg detailState.envVarsModel token detailState.app.name
                    in
                    ( { model
                        | view =
                            AppDetailView
                                { detailState
                                    | envVarsModel = updatedEnvVarsModel
                                    , error = maybeError
                                }
                      }
                    , Effect.map EnvVarsMsg envVarsEffect
                    )

                _ ->
                    ( model, Effect.none )

        ShowSettings ->
            case shared.token of
                Just token ->
                    ( { model
                        | view = SettingsView Settings.init
                        , activeSidebarItem = Settings
                      }
                    , Effect.FetchS3Config token (SettingsMsg << Settings.GotS3Config)
                    )

                Nothing ->
                    ( model, Effect.none )

        SettingsMsg settingsMsg ->
            case ( model.view, shared.token ) of
                ( SettingsView settingsState, Just token ) ->
                    let
                        ( newSettingsState, settingsEffect ) =
                            Settings.update settingsMsg settingsState token
                    in
                    ( { model | view = SettingsView newSettingsState }
                    , Effect.map SettingsMsg settingsEffect
                    )

                _ ->
                    ( model, Effect.none )

        HandleSSEEvent value ->
            -- Decode the unified SSE message and route to appropriate handler
            case Decode.decodeValue Data.unifiedSSEDecoder value of
                Ok (Data.GitHubOAuthMessage eventType data) ->
                    -- Route to GitHub OAuth handler
                    handleGitHubOAuthEvent model shared eventType data

                Ok (Data.BuildLogsMessage appName buildId eventType data) ->
                    -- Route to build logs handler
                    handleBuildLogsEvent model shared appName buildId eventType data

                Ok (Data.BuildStatusMessage appName buildId status) ->
                    -- Route to build status handler
                    handleBuildStatusEvent model shared appName buildId status

                Ok (Data.ContainerLogsMessage appName data) ->
                    -- Route to container logs handler
                    handleContainerLogsEvent model shared appName data

                Ok Data.HeartbeatMessage ->
                    -- Heartbeat, no action needed
                    ( model, Effect.none )

                Ok (Data.SystemNotificationMessage level message) ->
                    -- Could display a toast notification
                    ( model, Effect.none )

                Ok _ ->
                    -- Other message types not yet handled
                    ( model, Effect.none )

                Err _ ->
                    -- Failed to decode, ignore
                    ( model, Effect.none )


-- Note: GitHub SSE subscription is handled by Main.elm
-- Main subscribes to the sseEvent port and forwards events
-- to the Dashboard's HandleSSEEvent message when appropriate


-- SSE EVENT HANDLERS


handleGitHubOAuthEvent : Model -> Shared.Model navigationKey -> String -> String -> ( Model, Effect Msg )
handleGitHubOAuthEvent model shared eventType data =
    case ( model.view, shared.token ) of
        ( CreateAppView createState, Just token ) ->
            case eventType of
                "success" ->
                    -- Parse the data as JSON to get username
                    case Decode.decodeString (Decode.field "username" Decode.string) data of
                        Ok username ->
                            ( { model
                                | view =
                                    CreateAppView
                                        { createState
                                            | step = CreateApp.SelectRepo [] ""
                                            , error = Nothing
                                        }
                              }
                            , Effect.FetchRepos token GotRepoList
                            )

                        Err _ ->
                            -- Fallback: still a success, just no username parsed
                            ( { model
                                | view =
                                    CreateAppView
                                        { createState
                                            | step = CreateApp.SelectRepo [] ""
                                            , error = Nothing
                                        }
                              }
                            , Effect.FetchRepos token GotRepoList
                            )

                "error" ->
                    -- Stop polling and show error
                    case createState.step of
                        CreateApp.ConnectGitHub ghState ->
                            ( { model
                                | view =
                                    CreateAppView
                                        { createState
                                            | step = CreateApp.ConnectGitHub { ghState | polling = False }
                                            , error = Just data
                                        }
                              }
                            , Effect.none
                            )

                        _ ->
                            ( model, Effect.none )

                "pending" ->
                    -- Still waiting, keep polling
                    ( model, Effect.none )

                _ ->
                    -- Unknown event type, ignore
                    ( model, Effect.none )

        _ ->
            ( model, Effect.none )


handleBuildLogsEvent : Model -> Shared.Model navigationKey -> String -> String -> String -> String -> ( Model, Effect Msg )
handleBuildLogsEvent model shared _ _ eventType data =
    case ( model.view, shared.token ) of
        ( AppDetailView detailState, Just token ) ->
            case eventType of
                "message" ->
                    -- Append log line
                    let
                        newLogs =
                            if String.isEmpty detailState.buildLogs then
                                data

                            else
                                detailState.buildLogs ++ "\n" ++ data
                    in
                    ( { model
                        | view =
                            AppDetailView
                                { detailState | buildLogs = newLogs }
                      }
                    , Effect.none
                    )

                "done" ->
                    -- Build completed, refresh
                    ( { model
                        | view =
                            AppDetailView
                                { detailState
                                    | actionInProgress = Nothing
                                    , buildLogsStreaming = False
                                    , streamingBuildId = Nothing
                                }
                      }
                    , Effect.batch
                        [ Effect.FetchAppDetail token detailState.app.name GotAppDetail
                        , Effect.FetchBuilds token detailState.app.name GotBuilds
                        ]
                    )

                "error" ->
                    -- Error occurred - clear action in progress and refresh app state
                    -- token is already available from the outer case match
                    ( { model
                        | view =
                            AppDetailView
                                { detailState
                                    | actionInProgress = Nothing
                                    , buildLogsStreaming = False
                                    , streamingBuildId = Nothing
                                    , error = Just ("Build error: " ++ data)
                                }
                      }
                    , Effect.batch
                        [ Effect.FetchAppDetail token detailState.app.name GotAppDetail
                        , Effect.FetchBuilds token detailState.app.name GotBuilds
                        ]
                    )

                _ ->
                    ( model, Effect.none )

        _ ->
            ( model, Effect.none )


handleBuildStatusEvent : Model -> Shared.Model navigationKey -> String -> String -> String -> ( Model, Effect Msg )
handleBuildStatusEvent model shared appName _ status =
    -- Handle build status changes (building, success, failed)
    case status of
        "failed" ->
            -- Build failed - update state and clear action in progress
            let
                -- Update apps list immediately for responsive UI
                updatedApps =
                    List.map
                        (\app ->
                            if app.name == appName then
                                { app | state = "failed" }
                            else
                                app
                        )
                        model.apps

                -- Update app detail view if we're viewing this app
                ( updatedView, refreshEffect ) =
                    case model.view of
                        AppDetailView detailState ->
                            if detailState.app.name == appName then
                                case shared.token of
                                    Just token ->
                                        ( AppDetailView
                                            { detailState
                                                | actionInProgress = Nothing
                                            }
                                        , Effect.batch
                                            [ Effect.FetchAppDetail token appName GotAppDetail
                                            , Effect.FetchBuilds token appName GotBuilds
                                            ]
                                        )

                                    Nothing ->
                                        ( AppDetailView
                                            { detailState
                                                | actionInProgress = Nothing
                                            }
                                        , Effect.none
                                        )

                            else
                                ( model.view, Effect.none )

                        AppsListView ->
                            -- Refresh apps list to ensure we have latest state from server
                            case shared.token of
                                Just token ->
                                    ( model.view
                                    , Effect.FetchApps token GotApps
                                    )

                                Nothing ->
                                    ( model.view, Effect.none )

                        _ ->
                            ( model.view, Effect.none )
            in
            ( { model
                | apps = updatedApps
                , view = updatedView
              }
            , refreshEffect
            )

        "success" ->
            -- Build succeeded - update state and clear action in progress
            let
                -- Update apps list immediately for responsive UI
                updatedApps =
                    List.map
                        (\app ->
                            if app.name == appName then
                                -- If app was in "building" state, it should now be "stopped" or previous state
                                { app | state = if app.state == "building" then "stopped" else app.state }
                            else
                                app
                        )
                        model.apps

                -- Update app detail view if we're viewing this app
                ( updatedView, refreshEffect ) =
                    case model.view of
                        AppDetailView detailState ->
                            if detailState.app.name == appName then
                                case shared.token of
                                    Just token ->
                                        ( AppDetailView
                                            { detailState
                                                | actionInProgress = Nothing
                                            }
                                        , Effect.batch
                                            [ Effect.FetchAppDetail token appName GotAppDetail
                                            , Effect.FetchBuilds token appName GotBuilds
                                            ]
                                        )

                                    Nothing ->
                                        ( AppDetailView
                                            { detailState
                                                | actionInProgress = Nothing
                                            }
                                        , Effect.none
                                        )

                            else
                                ( model.view, Effect.none )

                        AppsListView ->
                            -- Refresh apps list to ensure we have latest state from server
                            case shared.token of
                                Just token ->
                                    ( model.view
                                    , Effect.FetchApps token GotApps
                                    )

                                Nothing ->
                                    ( model.view, Effect.none )

                        _ ->
                            ( model.view, Effect.none )
            in
            ( { model
                | apps = updatedApps
                , view = updatedView
              }
            , refreshEffect
            )

        _ ->
            -- Other statuses (e.g., "building") - no action needed yet
            ( model, Effect.none )


handleContainerLogsEvent : Model -> Shared.Model navigationKey -> String -> String -> ( Model, Effect Msg )
handleContainerLogsEvent model shared appName data =
    case ( model.view, shared.token ) of
        ( AppDetailView detailState, Just token ) ->
            -- Only handle logs for the app we're currently viewing
            if detailState.app.name == appName then
                -- Append log line
                let
                    newLogs =
                        if String.isEmpty detailState.logs then
                            data

                        else
                            detailState.logs ++ "\n" ++ data
                in
                ( { model
                    | view =
                        AppDetailView
                            { detailState | logs = newLogs }
                  }
                , Effect.none
                )

            else
                ( model, Effect.none )

        _ ->
            ( model, Effect.none )


-- TESTABLE HELPERS


mapCreateAppMsg : CreateApp.Msg -> Msg
mapCreateAppMsg createAppMsg =
    case createAppMsg of
        CreateApp.ShowCreateApp ->
            ShowCreateApp

        CreateApp.CancelCreateApp ->
            CancelCreateApp

        CreateApp.AppNameChanged name ->
            AppNameChanged name

        CreateApp.SubmitAppName ->
            SubmitAppName

        CreateApp.StartGitHubConnect ->
            StartGitHubConnect

        CreateApp.GotDeviceFlowStart result ->
            GotDeviceFlowStart result

        CreateApp.GotRepoList result ->
            GotRepoList result

        CreateApp.RepoSearchChanged query ->
            RepoSearchChanged query

        CreateApp.ChooseRepo repo ->
            ChooseRepo repo

        CreateApp.SkipRepoSelection ->
            SkipRepoSelection


{-| Pure function to handle GitHub status response. Extracted for testability.
Takes the GitHub status result, current create state, and auth token.
Returns the new create state step, any error message, and effects to emit.
-}
handleGotGitHubStatus :
    Result String Effect.GitHubStatusResponse
    -> CreateAppState
    -> Maybe String
    -> ( CreateAppStep, Maybe String, Effect Msg )
handleGotGitHubStatus result createState maybeToken =
    case ( result, maybeToken ) of
        ( Ok response, Just token ) ->
            if response.connected then
                let
                    username =
                        Maybe.withDefault "" response.username

                    status =
                        Effect.GitHubConnected username
                in
                ( CreateApp.SelectRepo [] ""
                , Nothing
                , Effect.batch
                    [ Effect.UpdateGitHubStatus status
                    , Effect.FetchRepos token GotRepoList
                    ]
                )

            else
                ( CreateApp.ConnectGitHub
                    { userCode = ""
                    , verificationUri = ""
                    , deviceCode = ""
                    , expiresIn = 0
                    , interval = 0
                    , polling = False
                    }
                , Nothing
                , Effect.UpdateGitHubStatus Effect.GitHubNotConnected
                )

        ( Err _, _ ) ->
            ( CreateApp.EnterName
            , Just "Failed to check GitHub connection"
            , Effect.none
            )

        ( Ok _, Nothing ) ->
            -- No token, can't proceed
            ( createState.step
            , Nothing
            , Effect.none
            )


-- VIEW


viewError : Maybe String -> Html Msg
viewError maybeError =
    case maybeError of
        Just error ->
            div [ class "bg-litehouse-error/10 text-litehouse-error p-3 rounded-xl mb-4 text-sm text-left" ] [ text error ]

        Nothing ->
            text ""


view : Shared.Model navigationKey -> Model -> Html Msg
view shared model =
    div []
        [ case model.view of
            AppsListView ->
                viewAppsList model

            CreateAppView createState ->
                Html.map (\createAppMsg -> mapCreateAppMsg createAppMsg) (CreateApp.view shared createState)

            AppDetailView detailState ->
                viewAppDetail detailState

            SettingsView settingsState ->
                Html.map SettingsMsg (Settings.view shared settingsState)
        ]


viewAppsList : Model -> Html Msg
viewAppsList model =
    div []
        [ div [ class "flex justify-between items-center mb-6" ]
            [ h2 [ class "text-xl font-semibold text-litehouse-text" ] [ text "My Apps" ]
            ]
        , if model.appsLoading then
            div [ class "flex flex-col items-center justify-center py-16 text-litehouse-muted" ]
                [ div [ class "w-10 h-10 border-3 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow mb-4" ] []
                , p [] [ text "Loading apps..." ]
                ]

          else if List.isEmpty model.apps then
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
                (List.map viewAppCard model.apps)
        ]


viewAppCard : Effect.AppInfo -> Html Msg
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
                , div []
                    [ span [ class "block text-xs text-litehouse-muted uppercase font-medium mb-1" ] [ text "URL" ]
                    , a
                        [ href ("https://" ++ app.name ++ ".litehouse.run")
                        , target "_blank"
                        , class "text-sm text-litehouse-amber hover:text-litehouse-amberDeep hover:underline break-all"
                        ]
                        [ text (app.name ++ ".litehouse.run") ]
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

        -- Environment Variables section
        , div [ class "bg-litehouse-surface rounded-2xl shadow-soft border border-litehouse-border p-6" ]
            [ h3 [ class "text-base font-semibold text-litehouse-text mb-4" ] [ text "Environment Variables" ]
            , Html.map EnvVarsMsg (EnvVars.view state.envVarsModel)
            ]

        -- Logs section
        , div [ class "bg-litehouse-surface rounded-2xl shadow-soft border border-litehouse-border p-6" ]
            [ div [ class "flex justify-between items-center mb-4" ]
                [ h3 [ class "text-base font-semibold text-litehouse-text" ] [ text "Logs" ]
                , div [ class "flex items-center gap-2" ]
                    [ -- Tab buttons
                      div [ class "flex rounded-lg border border-litehouse-border overflow-hidden" ]
                        [ button
                            [ class
                                (if state.logsView == BuildLogs then
                                    "px-3 py-1.5 text-sm bg-litehouse-amber text-white"

                                 else
                                    "px-3 py-1.5 text-sm text-litehouse-muted hover:bg-litehouse-bg"
                                )
                            , onClick (SwitchLogsView BuildLogs)
                            ]
                            [ text "Build" ]
                        , button
                            [ class
                                (if state.logsView == RuntimeLogs then
                                    "px-3 py-1.5 text-sm bg-litehouse-amber text-white"

                                 else
                                    "px-3 py-1.5 text-sm text-litehouse-muted hover:bg-litehouse-bg"
                                )
                            , onClick (SwitchLogsView RuntimeLogs)
                            ]
                            [ text "App" ]
                        ]

                    -- Refresh button (only for runtime logs)
                    , if state.logsView == RuntimeLogs then
                        button
                            [ class "px-3 py-1.5 text-sm border border-litehouse-border text-litehouse-muted hover:bg-litehouse-bg rounded-xl transition-colors disabled:opacity-50"
                            , onClick FetchLogs
                            , disabled state.logsLoading
                            ]
                            [ text "Refresh" ]

                      else
                        text ""
                    ]
                ]

            -- Build selector (only shown when viewing build logs)
            , if state.logsView == BuildLogs && not (List.isEmpty state.builds) then
                div [ class "mb-4" ]
                    [ select
                        [ class "w-full px-3 py-2 bg-litehouse-bg border border-litehouse-border rounded-xl text-litehouse-text text-sm focus:outline-none focus:ring-2 focus:ring-litehouse-amber"
                        , onInput SelectBuild
                        ]
                        (List.map
                            (\build ->
                                option
                                    [ value build.id
                                    , selected (state.selectedBuildId == Just build.id)
                                    ]
                                    [ text (String.left 8 (Maybe.withDefault "unknown" build.gitCommit) ++ " - " ++ Maybe.withDefault "no tag" build.imageTag ++ " (" ++ formatBuildDate build.createdAt ++ ")") ]
                            )
                            state.builds
                        )
                    ]

              else
                text ""

            -- Log content
            , viewLogsContent state
            ]
        ]


viewLogsContent : AppDetailState -> Html Msg
viewLogsContent state =
    case state.logsView of
        RuntimeLogs ->
            if state.logsLoading then
                div [ class "flex items-center justify-center gap-3 py-10 text-litehouse-muted" ]
                    [ div [ class "w-5 h-5 border-2 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow" ] []
                    , span [] [ text "Loading logs..." ]
                    ]

            else if String.isEmpty state.logs then
                div [ class "py-10 text-center text-litehouse-muted" ]
                    [ text "No runtime logs available" ]

            else
                pre [ class "bg-gray-900 text-gray-300 font-mono text-xs p-4 rounded-xl overflow-auto max-h-96 whitespace-pre-wrap break-all" ]
                    [ text state.logs ]

        BuildLogs ->
            if state.buildLogsStreaming then
                -- Show streaming build logs with indicator
                div []
                    [ div [ class "flex items-center gap-2 mb-2 text-litehouse-warning" ]
                        [ div [ class "w-4 h-4 border-2 border-litehouse-warning/30 border-t-litehouse-warning rounded-full animate-spin-slow" ] []
                        , span [ class "text-sm font-medium" ] [ text "Building..." ]
                        ]
                    , pre [ class "bg-gray-900 text-gray-300 font-mono text-xs p-4 rounded-xl overflow-auto max-h-96 whitespace-pre-wrap break-all", id "build-logs-stream" ]
                        [ text
                            (if String.isEmpty state.buildLogs then
                                "Waiting for build output..."

                             else
                                state.buildLogs
                            )
                        ]
                    ]

            else if state.buildLogsLoading then
                div [ class "flex items-center justify-center gap-3 py-10 text-litehouse-muted" ]
                    [ div [ class "w-5 h-5 border-2 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow" ] []
                    , span [] [ text "Loading build logs..." ]
                    ]

            else if List.isEmpty state.builds && state.streamingBuildId == Nothing then
                div [ class "py-10 text-center text-litehouse-muted" ]
                    [ text "No builds yet. Click Build to create your first build." ]

            else if String.isEmpty state.buildLogs then
                div [ class "py-10 text-center text-litehouse-muted" ]
                    [ text "No logs available for this build" ]

            else
                pre [ class "bg-gray-900 text-gray-300 font-mono text-xs p-4 rounded-xl overflow-auto max-h-96 whitespace-pre-wrap break-all" ]
                    [ text state.buildLogs ]


formatBuildDate : String -> String
formatBuildDate isoDate =
    -- Simple formatting: just take the date part (first 10 characters)
    String.left 10 isoDate



-- PAGE LIFECYCLE


{-| Save page state to global state when leaving the page
-}
save : Model -> Shared.Model navigationKey -> Shared.Model navigationKey
save model shared =
    -- GitHub status is already saved in Shared via update messages
    shared


{-| Load data from global state when entering the page
-}
load : Shared.Model navigationKey -> Model -> Model
load shared model =
    -- Nothing specific to load, token and user come from Shared
    model
