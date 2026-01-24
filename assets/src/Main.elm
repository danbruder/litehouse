port module Main exposing (main, init, initForTesting, update, view, Model, Msg(..), Flags)

import Browser
import Browser.Navigation as Nav
import Effect
import Html exposing (Html, div, p, text)
import Html.Attributes exposing (class)
import Http
import Json.Decode as Decode
import Json.Encode as Encode
import Page.Dashboard
import Page.Login
import Page.Setup
import Route
import Shared

import Time

import Url


-- PORTS


port saveToken : String -> Cmd msg


port saveRefreshToken : String -> Cmd msg


port clearToken : () -> Cmd msg


port getRefreshToken : () -> Cmd msg


port refreshTokenReceived : (Maybe String -> msg) -> Sub msg


-- Unified SSE ports
port connectSSE : { token : String, filters : Maybe Encode.Value } -> Cmd msg


port disconnectSSE : () -> Cmd msg


port sseEvent : (Decode.Value -> msg) -> Sub msg


port sseConnectionState : (String -> msg) -> Sub msg



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


type alias Model navigationKey =
    { shared : Shared.Model
    , page : Page
    }


type Page
    = NotFound
    | Loading
    | Login Page.Login.Model
    | Setup Page.Setup.Model
    | Dashboard Page.Dashboard.Model





-- INIT


init : Flags -> Url.Url -> Nav.Key -> ( Model Nav.Key, Cmd Msg )
init flags url navKey =
    let
        route =
            Route.fromUrl url

        shared =
            Shared.init navKey
                |> (\s -> { s | currentRoute = route })

        initialModel =
            { shared = shared
            , page = Loading
            }
    in
    case flags.token of
        Just token ->
            -- We have a token, verify it
            ( initialModel
            , verifyTokenHttp token
            )

        Nothing ->
            -- No token, check server status
            ( initialModel
            , checkServerStatusHttp
            )


initForTesting : Flags -> Url.Url -> () -> ( Model (), Cmd Msg )
initForTesting flags url () =
    -- Test version - ProgramTest.createApplication expects init to take () instead of Nav.Key
    -- Based on NavigationKeyExample pattern: use () as the navigationKey type in tests
    -- This allows ProgramTest to handle navigation internally
    init flags url ()



-- UPDATE


type Msg
    = UrlChanged Url.Url
    | LinkClicked Browser.UrlRequest
    | SharedMsg Shared.Msg
    | LoginMsg Page.Login.Msg
    | SetupMsg Page.Setup.Msg
    | DashboardMsg Page.Dashboard.Msg
    | GotServerStatus (Result Http.Error ServerStatus)
    | GotTokenVerification (Result Http.Error TokenVerificationResponse)
    | RefreshTokenReceived (Maybe String)
    | GotTokenRefresh (Result Http.Error TokenPair)
    | GotGitHubPollingStarted (Result Http.Error ())
    | SSEEvent Decode.Value
    | SSEConnectionStateChanged String


update : Msg -> Model Nav.Key -> ( Model Nav.Key, Cmd Msg )
update msg model =
    case msg of
        UrlChanged url ->
            let
                route =
                    Route.fromUrl url

                newShared =
                    Shared.update (Shared.SetRoute route) model.shared
            in
            case route of
                Just Route.Login ->
                    let
                        ( pageModel, effect ) =
                            Page.Login.init newShared
                    in
                    ( { model
                        | shared = newShared
                        , page = Login pageModel
                      }
                    , performEffect model.shared.navKey (Effect.map LoginMsg effect)
                    )

                Just Route.Setup ->
                    let
                        ( pageModel, effect ) =
                            Page.Setup.init newShared
                    in
                    ( { model
                        | shared = newShared
                        , page = Setup pageModel
                      }
                    , performEffect model.shared.navKey (Effect.map SetupMsg effect)
                    )

                Just Route.Dashboard ->
                    case model.page of
                        Dashboard _ ->
                            -- Already on dashboard, just update route
                            ( { model | shared = newShared }
                            , Cmd.none
                            )

                        _ ->
                            -- Initialize dashboard
                            let
                                ( pageModel, effect ) =
                                    Page.Dashboard.init newShared
                            in
                            ( { model
                                | shared = newShared
                                , page = Dashboard pageModel
                              }
                            , performEffect model.shared.navKey (Effect.map DashboardMsg effect)
                            )

                Just (Route.AppDetail appName) ->
                    case model.page of
                        Dashboard dashModel ->
                            -- Stay on dashboard, it will handle the app detail view
                            ( { model | shared = newShared }
                            , Cmd.none
                            )

                        _ ->
                            -- Not on dashboard, redirect
                            ( { model | shared = newShared }
                            , Cmd.none
                            )

                Nothing ->
                    ( { model
                        | shared = newShared
                        , page = NotFound
                      }
                    , Cmd.none
                    )

        LinkClicked urlRequest ->
            case urlRequest of
                Browser.Internal url ->
                    ( model
                    , Nav.pushUrl model.shared.navKey (Url.toString url)
                    )

                Browser.External href ->
                    ( model
                    , Nav.load href
                    )

        SharedMsg sharedMsg ->
            ( { model | shared = Shared.update sharedMsg model.shared }
            , Cmd.none
            )

        LoginMsg loginMsg ->
            case model.page of
                Login pageModel ->
                    let
                        ( newPageModel, effect ) =
                            Page.Login.update model.shared loginMsg pageModel

                        ( newModel, cmd ) =
                            handleLoginEffect model newPageModel effect
                    in
                    ( newModel, cmd )

                _ ->
                    ( model, Cmd.none )

        SetupMsg setupMsg ->
            case model.page of
                Setup pageModel ->
                    let
                        ( newPageModel, effect ) =
                            Page.Setup.update model.shared setupMsg pageModel

                        ( newModel, cmd ) =
                            handleSetupEffect model newPageModel effect
                    in
                    ( newModel, cmd )

                _ ->
                    ( model, Cmd.none )

        DashboardMsg dashboardMsg ->
            case model.page of
                Dashboard pageModel ->
                    let
                        ( newPageModel, effect ) =
                            Page.Dashboard.update model.shared dashboardMsg pageModel

                        ( newModel, cmd ) =
                            handleDashboardEffect model newPageModel effect
                    in
                    ( newModel, cmd )

                _ ->
                    ( model, Cmd.none )

        GotServerStatus result ->
            case result of
                Ok status ->
                    let
                        newShared =
                            Shared.update (Shared.SetServerVersion status.version) model.shared
                    in
                    if status.initialized then
                        let
                            ( pageModel, effect ) =
                                Page.Login.init newShared
                        in
                        ( { model
                            | shared = newShared
                            , page = Login pageModel
                          }
                        , Cmd.batch
                            [ Nav.pushUrl model.shared.navKey "/login"
                            , performEffect model.shared.navKey (Effect.map LoginMsg effect)
                            ]
                        )

                    else
                        let
                            ( pageModel, effect ) =
                                Page.Setup.init newShared
                        in
                        ( { model
                            | shared = newShared
                            , page = Setup pageModel
                          }
                        , Cmd.batch
                            [ Nav.pushUrl model.shared.navKey "/setup"
                            , performEffect model.shared.navKey (Effect.map SetupMsg effect)
                            ]
                        )

                Err _ ->
                    let
                        ( pageModel, effect ) =
                            Page.Login.init model.shared
                    in
                    ( { model | page = Login pageModel }
                    , Cmd.batch
                        [ Nav.pushUrl model.shared.navKey "/login"
                        , performEffect model.shared.navKey (Effect.map LoginMsg effect)
                        ]
                    )

        GotTokenVerification result ->
            case result of
                Ok response ->
                    -- Token is valid, update shared and get refresh token
                    let
                        newShared =
                            model.shared
                                |> Shared.update (Shared.SetUser response.user)
                                |> Shared.update (Shared.SetTokens response.token "")
                    in
                    ( { model | shared = newShared }
                    , getRefreshToken ()
                    )

                Err _ ->
                    -- Token invalid, try to refresh if we have a refresh token
                    ( model
                    , getRefreshToken ()
                    )

        RefreshTokenReceived maybeRefreshToken ->
            case maybeRefreshToken of
                Just refreshToken ->
                    case ( model.shared.token, model.shared.user ) of
                        ( Just token, Just user ) ->
                            -- We have both token and user, set up dashboard and connect SSE
                            let
                                newShared =
                                    Shared.update (Shared.SetTokens token refreshToken) model.shared

                                ( pageModel, effect ) =
                                    Page.Dashboard.init newShared
                            in
                            ( { model
                                | shared = newShared
                                , page = Dashboard pageModel
                              }
                            , Cmd.batch
                                [ Nav.pushUrl model.shared.navKey "/dashboard"
                                , saveRefreshToken refreshToken
                                , connectSSE { token = token, filters = Nothing }
                                , performEffect model.shared.navKey (Effect.map DashboardMsg effect)
                                ]
                            )

                        _ ->
                            -- No token/user, try to refresh access token
                            ( model
                            , refreshAccessTokenHttp refreshToken
                            )

                Nothing ->
                    -- No refresh token, clear and check status
                    ( { model | shared = Shared.update Shared.ClearAuth model.shared }
                    , Cmd.batch
                        [ clearToken ()
                        , disconnectSSE ()
                        , checkServerStatusHttp
                        ]
                    )

        GotTokenRefresh result ->
            case result of
                Ok tokenPair ->
                    -- We got new tokens, verify the access token
                    ( model
                    , Cmd.batch
                        [ saveToken tokenPair.accessToken
                        , saveRefreshToken tokenPair.refreshToken
                        , verifyTokenHttp tokenPair.accessToken
                        ]
                    )

                Err _ ->
                    -- Refresh failed, clear tokens and redirect to login
                    let
                        newShared =
                            Shared.update Shared.ClearAuth model.shared

                        ( pageModel, effect ) =
                            Page.Login.init newShared
                    in
                    ( { model
                        | shared = newShared
                        , page = Login pageModel
                      }
                    , Cmd.batch
                        [ Nav.pushUrl model.shared.navKey "/login"
                        , clearToken ()
                        , disconnectSSE ()
                        , performEffect model.shared.navKey (Effect.map LoginMsg effect)
                        ]
                    )

        SSEEvent value ->
            -- Route SSE message to appropriate page based on message type
            -- For now, forward all to Dashboard if we're on that page
            case model.page of
                Dashboard pageModel ->
                    let
                        ( newPageModel, effect ) =
                            Page.Dashboard.update model.shared (Page.Dashboard.HandleSSEEvent value) pageModel
                    in
                    ( { model | page = Dashboard newPageModel }
                    , performEffect model.shared.navKey (Effect.map DashboardMsg effect)
                    )

                _ ->
                    -- TODO: Handle SSE events on other pages or update global state
                    ( model, Cmd.none )

        GotGitHubPollingStarted _ ->
            -- Polling started, events will come via SSE
            ( model, Cmd.none )

        SSEConnectionStateChanged state ->
            let
                newShared =
                    Shared.update (Shared.SetSSEConnectionState state) model.shared
            in
            ( { model | shared = newShared }, Cmd.none )



-- HANDLE PAGE EFFECTS


handleLoginEffect : Model -> Page.Login.Model -> Effect.Effect Page.Login.Msg -> ( Model, Cmd Msg )
handleLoginEffect model pageModel effect =
    -- Check if effect contains auth response that we need to handle at main level
    case getAuthResponseFromEffect effect of
        Just authResponse ->
            -- Update shared with user and tokens
            let
                newShared =
                    model.shared
                        |> Shared.update (Shared.SetUser authResponse.user)
                        |> Shared.update (Shared.SetTokens authResponse.accessToken authResponse.refreshToken)

                ( dashModel, dashEffect ) =
                    Page.Dashboard.init newShared
            in
            ( { model
                | shared = newShared
                , page = Dashboard dashModel
              }
            , Cmd.batch
                [ performEffect model.shared.navKey (Effect.map LoginMsg effect)
                , performEffect model.shared.navKey (Effect.map DashboardMsg dashEffect)
                ]
            )

        Nothing ->
            ( { model | page = Login pageModel }
            , performEffect model.shared.navKey (Effect.map LoginMsg effect)
            )


handleSetupEffect : Model -> Page.Setup.Model -> Effect.Effect Page.Setup.Msg -> ( Model, Cmd Msg )
handleSetupEffect model pageModel effect =
    -- Check if effect contains auth response that we need to handle at main level
    case getAuthResponseFromEffect effect of
        Just authResponse ->
            -- Update shared with user and tokens
            let
                newShared =
                    model.shared
                        |> Shared.update (Shared.SetUser authResponse.user)
                        |> Shared.update (Shared.SetTokens authResponse.accessToken authResponse.refreshToken)

                ( dashModel, dashEffect ) =
                    Page.Dashboard.init newShared
            in
            ( { model
                | shared = newShared
                , page = Dashboard dashModel
              }
            , Cmd.batch
                [ performEffect model.shared.navKey (Effect.map SetupMsg effect)
                , performEffect model.shared.navKey (Effect.map DashboardMsg dashEffect)
                ]
            )

        Nothing ->
            ( { model | page = Setup pageModel }
            , performEffect model.shared.navKey (Effect.map SetupMsg effect)
            )


handleDashboardEffect : Model -> Page.Dashboard.Model -> Effect.Effect Page.Dashboard.Msg -> ( Model, Cmd Msg )
handleDashboardEffect model pageModel effect =
    -- Check if effect clears auth
    if containsClearToken effect then
        let
            newShared =
                Shared.update Shared.ClearAuth model.shared

            ( loginModel, loginEffect ) =
                Page.Login.init newShared
        in
        ( { model
            | shared = newShared
            , page = Login loginModel
          }
        , Cmd.batch
            [ performEffect model.shared.navKey (Effect.map DashboardMsg effect)
            , performEffect model.shared.navKey (Effect.map LoginMsg loginEffect)
            ]
        )

    else
        -- Check if GitHub status changed
        let
            newShared =
                updateSharedFromEffect model.shared effect
        in
        ( { model
            | shared = newShared
            , page = Dashboard pageModel
          }
        , performEffect model.shared.navKey (Effect.map DashboardMsg effect)
        )



-- EFFECT HELPERS


{-| Extract AuthResponse from an effect if it contains one
-}
getAuthResponseFromEffect : Effect.Effect msg -> Maybe Effect.AuthResponse
getAuthResponseFromEffect effect =
    -- This is a simplified check - in reality, effects are opaque
    -- We'll handle auth responses in performEffect instead
    Nothing


{-| Check if effect contains ClearToken
-}
containsClearToken : Effect.Effect msg -> Bool
containsClearToken effect =
    -- This is a simplified check - effects are opaque
    -- We'll handle logout in performEffect
    False


{-| Update shared model based on effect side effects
-}
updateSharedFromEffect : Shared.Model -> Effect.Effect msg -> Shared.Model
updateSharedFromEffect shared effect =
    case effect of
        Effect.UpdateGitHubStatus status ->
            let
                sharedStatus =
                    case status of
                        Effect.GitHubUnknown ->
                            Shared.GitHubUnknown

                        Effect.GitHubNotConnected ->
                            Shared.GitHubNotConnected

                        Effect.GitHubConnected username ->
                            Shared.GitHubConnected username
            in
            Shared.update (Shared.SetGitHubStatus sharedStatus) shared

        Effect.Batch effects ->
            List.foldl (\eff acc -> updateSharedFromEffect acc eff) shared effects

        _ ->
            shared



-- PERFORM EFFECT


performEffect : Nav.Key -> Effect.Effect Msg -> Cmd Msg
performEffect navKey effect =
    case effect of
        Effect.None ->
            Cmd.none

        Effect.Batch effects ->
            Cmd.batch (List.map (performEffect navKey) effects)

        Effect.PushUrl url ->
            Nav.pushUrl navKey url

        Effect.ReplaceUrl url ->
            Nav.replaceUrl navKey url

        Effect.SaveToken token ->
            saveToken token

        Effect.SaveRefreshToken token ->
            saveRefreshToken token

        Effect.ClearToken ->
            Cmd.batch [ clearToken (), disconnectSSE () ]

        Effect.GetRefreshToken ->
            getRefreshToken ()

        Effect.ConnectSSE config ->
            connectSSE config

        Effect.DisconnectSSE ->
            disconnectSSE ()

        Effect.CheckServerStatus toMsg ->
            checkServerStatusHttp

        Effect.VerifyToken token toMsg ->
            verifyTokenHttp token

        Effect.RefreshAccessToken refreshToken toMsg ->
            refreshAccessTokenHttp refreshToken

        Effect.SubmitLogin form toMsg ->
            submitLoginHttp form toMsg

        Effect.SubmitRegister form toMsg ->
            submitRegisterHttp form toMsg

        Effect.FetchApps token toMsg ->
            fetchAppsHttp token toMsg

        Effect.FetchAppDetail token appName toMsg ->
            fetchAppDetailHttp token appName toMsg

        Effect.StartApp token appName toMsg ->
            startAppHttp token appName toMsg

        Effect.StopApp token appName toMsg ->
            stopAppHttp token appName toMsg

        Effect.BuildApp token appName toMsg ->
            buildAppHttp token appName toMsg

        Effect.DeleteApp token appName toMsg ->
            deleteAppHttp token appName toMsg

        Effect.FetchLogs token appName toMsg ->
            fetchLogsHttp token appName toMsg

        Effect.FetchBuilds token appName toMsg ->
            fetchBuildsHttp token appName toMsg

        Effect.FetchBuildLogs token appName buildId toMsg ->
            fetchBuildLogsHttp token appName buildId toMsg

        Effect.FetchGitHubStatus token toMsg ->
            fetchGitHubStatusHttp token toMsg

        Effect.StartDeviceFlow token toMsg ->
            startDeviceFlowHttp token toMsg

        Effect.StartGitHubPolling token deviceCode interval expiresIn ->
            startGitHubPollingHttp token deviceCode interval expiresIn

        Effect.FetchRepos token toMsg ->
            fetchReposHttp token toMsg

        Effect.CreateApp token name toMsg ->
            createAppHttp token name toMsg

        Effect.CreateAppWithRepo token name repo toMsg ->
            createAppWithRepoHttp token name repo toMsg

        Effect.UpdateGitHubStatus _ ->
            -- Handled by updateSharedFromEffect, no Cmd needed
            Cmd.none



-- HTTP REQUEST FUNCTIONS


checkServerStatusHttp : Cmd Msg
checkServerStatusHttp =
    Http.get
        { url = "/api/auth/status"
        , expect = Http.expectJson GotServerStatus serverStatusDecoder
        }


verifyTokenHttp : String -> Cmd Msg
verifyTokenHttp token =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/auth/me"
        , body = Http.emptyBody
        , expect = Http.expectJson GotTokenVerification (tokenVerificationDecoder token)
        , timeout = Nothing
        , tracker = Nothing
        }


refreshAccessTokenHttp : String -> Cmd Msg
refreshAccessTokenHttp refreshToken =
    Http.post
        { url = "/api/auth/refresh"
        , body =
            Http.jsonBody
                (Encode.object
                    [ ( "refresh_token", Encode.string refreshToken )
                    ]
                )
        , expect = Http.expectJson GotTokenRefresh tokenPairDecoder
        }


submitLoginHttp : Effect.LoginForm -> (Result String Effect.AuthResponse -> msg) -> Cmd msg
submitLoginHttp form toMsg =
    Http.post
        { url = "/api/auth/login"
        , body =
            Http.jsonBody
                (Encode.object
                    [ ( "email", Encode.string form.email )
                    , ( "password", Encode.string form.password )
                    ]
                )
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) authResponseDecoder
        }


submitRegisterHttp : Effect.SetupForm -> (Result String Effect.AuthResponse -> msg) -> Cmd msg
submitRegisterHttp form toMsg =
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
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) authResponseDecoder
        }


fetchAppsHttp : String -> (Result String (List Effect.AppInfo) -> msg) -> Cmd msg
fetchAppsHttp token toMsg =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps"
        , body = Http.emptyBody
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) appsListDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


fetchGitHubStatusHttp : String -> (Result String Effect.GitHubStatusResponse -> msg) -> Cmd msg
fetchGitHubStatusHttp token toMsg =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/github/status"
        , body = Http.emptyBody
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) githubStatusDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


startDeviceFlowHttp : String -> (Result String Effect.DeviceFlowStartResponse -> msg) -> Cmd msg
startDeviceFlowHttp token toMsg =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/github/connect/start"
        , body = Http.emptyBody
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) deviceFlowStartDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


startGitHubPollingHttp : String -> String -> Int -> Int -> Cmd Msg
startGitHubPollingHttp token deviceCode interval expiresIn =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/github/connect/stream?device_code=" ++ deviceCode ++ "&interval=" ++ String.fromInt interval ++ "&expires_in=" ++ String.fromInt expiresIn
        , body = Http.emptyBody
        , expect = Http.expectWhatever GotGitHubPollingStarted
        , timeout = Nothing
        , tracker = Nothing
        }


fetchReposHttp : String -> (Result String (List Effect.RepoInfo) -> msg) -> Cmd msg
fetchReposHttp token toMsg =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/github/repos"
        , body = Http.emptyBody
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) reposListDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


createAppHttp : String -> String -> (Result String Effect.AppInfo -> msg) -> Cmd msg
createAppHttp token name toMsg =
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
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) appInfoDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


createAppWithRepoHttp : String -> String -> String -> (Result String Effect.AppInfo -> msg) -> Cmd msg
createAppWithRepoHttp token name repoFullName toMsg =
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
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) appInfoDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


fetchAppDetailHttp : String -> String -> (Result String Effect.AppDetail -> msg) -> Cmd msg
fetchAppDetailHttp token appName toMsg =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName
        , body = Http.emptyBody
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) appDetailDecoder
        , timeout = Nothing
        , tracker = Nothing
        }


startAppHttp : String -> String -> (Result String String -> msg) -> Cmd msg
startAppHttp token appName toMsg =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/start"
        , body = Http.emptyBody
        , expect = Http.expectString (toMsg << Result.mapError httpErrorToString)
        , timeout = Nothing
        , tracker = Nothing
        }


stopAppHttp : String -> String -> (Result String String -> msg) -> Cmd msg
stopAppHttp token appName toMsg =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/stop"
        , body = Http.emptyBody
        , expect = Http.expectString (toMsg << Result.mapError httpErrorToString)
        , timeout = Nothing
        , tracker = Nothing
        }


buildAppHttp : String -> String -> (Result String String -> msg) -> Cmd msg
buildAppHttp token appName toMsg =
    Http.request
        { method = "POST"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/build"
        , body = Http.emptyBody
        , expect = Http.expectString (toMsg << Result.mapError httpErrorToString)
        , timeout = Just 300000
        , tracker = Nothing
        }


deleteAppHttp : String -> String -> (Result String String -> msg) -> Cmd msg
deleteAppHttp token appName toMsg =
    Http.request
        { method = "DELETE"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName
        , body = Http.emptyBody
        , expect = Http.expectString (toMsg << Result.mapError httpErrorToString)
        , timeout = Nothing
        , tracker = Nothing
        }


fetchLogsHttp : String -> String -> (Result String String -> msg) -> Cmd msg
fetchLogsHttp token appName toMsg =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/logs?lines=100"
        , body = Http.emptyBody
        , expect = Http.expectString (toMsg << Result.mapError httpErrorToString)
        , timeout = Nothing
        , tracker = Nothing
        }


fetchBuildsHttp : String -> String -> (Result String (List Effect.BuildInfo) -> msg) -> Cmd msg
fetchBuildsHttp token appName toMsg =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/builds"
        , body = Http.emptyBody
        , expect = Http.expectJson (toMsg << Result.mapError httpErrorToString) (Decode.list buildInfoDecoder)
        , timeout = Nothing
        , tracker = Nothing
        }


fetchBuildLogsHttp : String -> String -> String -> (Result String String -> msg) -> Cmd msg
fetchBuildLogsHttp token appName buildId toMsg =
    Http.request
        { method = "GET"
        , headers = [ Http.header "Authorization" ("Bearer " ++ token) ]
        , url = "/api/apps/" ++ appName ++ "/builds/" ++ buildId ++ "/logs"
        , body = Http.emptyBody
        , expect = Http.expectString (toMsg << Result.mapError httpErrorToString)
        , timeout = Nothing
        , tracker = Nothing
        }



-- DECODERS


type alias ServerStatus =
    { initialized : Bool
    , version : String
    }


serverStatusDecoder : Decode.Decoder ServerStatus
serverStatusDecoder =
    Decode.map2 ServerStatus
        (Decode.field "initialized" Decode.bool)
        (Decode.field "version" Decode.string)


buildInfoDecoder : Decode.Decoder Effect.BuildInfo
buildInfoDecoder =
    Decode.map6 Effect.BuildInfo
        (Decode.field "id" Decode.string)
        (Decode.field "app_id" Decode.string)
        (Decode.field "image_tag" (Decode.nullable Decode.string))
        (Decode.field "git_commit" (Decode.nullable Decode.string))
        (Decode.field "status" Decode.string)
        (Decode.field "created_at" Decode.string)


type alias TokenVerificationResponse =
    { user : Effect.UserInfo
    , token : String
    }


tokenVerificationDecoder : String -> Decode.Decoder TokenVerificationResponse
tokenVerificationDecoder token =
    Decode.map2 TokenVerificationResponse
        (Decode.map2 Effect.UserInfo
            (Decode.at [ "user", "email" ] Decode.string)
            (Decode.at [ "user", "full_name" ] Decode.string)
        )
        (Decode.succeed token)


authResponseDecoder : Decode.Decoder Effect.AuthResponse
authResponseDecoder =
    Decode.map3 Effect.AuthResponse
        (Decode.at [ "tokens", "access_token" ] Decode.string)
        (Decode.at [ "tokens", "refresh_token" ] Decode.string)
        (Decode.field "user" userDecoder)


type alias TokenPair =
    { accessToken : String
    , refreshToken : String
    }


tokenPairDecoder : Decode.Decoder TokenPair
tokenPairDecoder =
    Decode.map2 TokenPair
        (Decode.field "access_token" Decode.string)
        (Decode.field "refresh_token" Decode.string)


userDecoder : Decode.Decoder Effect.UserInfo
userDecoder =
    Decode.map2 Effect.UserInfo
        (Decode.field "email" Decode.string)
        (Decode.field "full_name" Decode.string)


appsListDecoder : Decode.Decoder (List Effect.AppInfo)
appsListDecoder =
    Decode.list appInfoDecoder


appInfoDecoder : Decode.Decoder Effect.AppInfo
appInfoDecoder =
    Decode.map3 Effect.AppInfo
        (Decode.field "id" Decode.string)
        (Decode.field "name" Decode.string)
        (Decode.field "state" Decode.string)


githubStatusDecoder : Decode.Decoder Effect.GitHubStatusResponse
githubStatusDecoder =
    Decode.map2 Effect.GitHubStatusResponse
        (Decode.field "connected" Decode.bool)
        (Decode.maybe (Decode.field "username" Decode.string))


deviceFlowStartDecoder : Decode.Decoder Effect.DeviceFlowStartResponse
deviceFlowStartDecoder =
    Decode.map5 Effect.DeviceFlowStartResponse
        (Decode.field "user_code" Decode.string)
        (Decode.field "verification_uri" Decode.string)
        (Decode.field "device_code" Decode.string)
        (Decode.field "expires_in" Decode.int)
        (Decode.field "interval" Decode.int)


reposListDecoder : Decode.Decoder (List Effect.RepoInfo)
reposListDecoder =
    Decode.list repoInfoDecoder


repoInfoDecoder : Decode.Decoder Effect.RepoInfo
repoInfoDecoder =
    Decode.map6 Effect.RepoInfo
        (Decode.field "name" Decode.string)
        (Decode.field "full_name" Decode.string)
        (Decode.maybe (Decode.field "description" Decode.string))
        (Decode.field "private" Decode.bool)
        (Decode.field "clone_url" Decode.string)
        (Decode.field "default_branch" Decode.string)


appDetailDecoder : Decode.Decoder Effect.AppDetail
appDetailDecoder =
    Decode.map7 Effect.AppDetail
        (Decode.field "id" Decode.string)
        (Decode.field "name" Decode.string)
        (Decode.field "state" Decode.string)
        (Decode.maybe (Decode.field "port" Decode.int))
        (Decode.field "created_at" Decode.string)
        (Decode.field "updated_at" Decode.string)
        (Decode.maybe (Decode.field "remote" remoteInfoDecoder))


remoteInfoDecoder : Decode.Decoder Effect.RemoteInfo
remoteInfoDecoder =
    Decode.map3 Effect.RemoteInfo
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


view : Model Nav.Key -> Browser.Document Msg
view model =
    { title = getPageTitle model.page
    , body =
        [ case model.page of
            NotFound ->
                viewNotFound

            Loading ->
                viewLoading

            Login pageModel ->
                Page.Login.view model.shared pageModel
                    |> Html.map LoginMsg

            Setup pageModel ->
                Page.Setup.view model.shared pageModel
                    |> Html.map SetupMsg

            Dashboard pageModel ->
                Page.Dashboard.view model.shared pageModel
                    |> Html.map DashboardMsg
        ]
    }


getPageTitle : Page -> String
getPageTitle page =
    case page of
        NotFound ->
            "Not Found - Litehouse"

        Loading ->
            "Loading - Litehouse"

        Login _ ->
            "Login - Litehouse"

        Setup _ ->
            "Setup - Litehouse"

        Dashboard _ ->
            "Dashboard - Litehouse"


viewNotFound : Html Msg
viewNotFound =
    div [ class "min-h-screen bg-litehouse-bg flex items-center justify-center" ]
        [ div [ class "bg-litehouse-surface rounded-2xl shadow-soft p-10 text-center" ]
            [ p [ class "text-litehouse-muted" ] [ text "Page not found" ]
            ]
        ]


viewLoading : Html Msg
viewLoading =
    div [ class "min-h-screen bg-litehouse-bg flex items-center justify-center" ]
        [ div [ class "bg-litehouse-surface rounded-2xl shadow-soft p-10 text-center" ]
            [ div [ class "w-10 h-10 border-3 border-litehouse-border border-t-litehouse-amber rounded-full animate-spin-slow mx-auto mb-4" ] []
            , p [ class "text-litehouse-muted" ] [ text "Loading..." ]
            ]
        ]



-- SUBSCRIPTIONS


subscriptions : Model -> Sub Msg
subscriptions model =
    Sub.batch
        [ refreshTokenReceived RefreshTokenReceived
        , sseEvent SSEEvent
        , sseConnectionState SSEConnectionStateChanged
        ]
