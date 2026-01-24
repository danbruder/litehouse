module Shared exposing
    ( Model
    , Msg(..)
    , UserInfo
    , GitHubStatus(..)
    , init
    , update
    )

import Browser.Navigation as Nav
import Route exposing (Route)


-- MODEL


type alias Model navigationKey =
    { navKey : navigationKey
    , currentRoute : Maybe Route
    , serverVersion : String
    , user : Maybe UserInfo
    , token : Maybe String
    , refreshToken : Maybe String
    , githubStatus : GitHubStatus
    , sseConnectionState : String
    }


type alias UserInfo =
    { email : String
    , fullName : String
    }


type GitHubStatus
    = GitHubUnknown
    | GitHubNotConnected
    | GitHubConnected String


-- INIT


init : navigationKey -> Model navigationKey
init navKey =
    { navKey = navKey
    , currentRoute = Nothing
    , serverVersion = ""
    , user = Nothing
    , token = Nothing
    , refreshToken = Nothing
    , githubStatus = GitHubUnknown
    , sseConnectionState = "disconnected"
    }


-- UPDATE


type Msg
    = SetRoute (Maybe Route)
    | SetServerVersion String
    | SetUser UserInfo
    | SetTokens String String
    | ClearAuth
    | SetGitHubStatus GitHubStatus
    | SetSSEConnectionState String


update : Msg -> Model navigationKey -> Model navigationKey
update msg model =
    case msg of
        SetRoute route ->
            { model | currentRoute = route }

        SetServerVersion version ->
            { model | serverVersion = version }

        SetUser user ->
            { model | user = Just user }

        SetTokens token refreshToken ->
            { model
                | token = Just token
                , refreshToken = Just refreshToken
            }

        ClearAuth ->
            { model
                | user = Nothing
                , token = Nothing
                , refreshToken = Nothing
                , githubStatus = GitHubUnknown
            }

        SetGitHubStatus status ->
            { model | githubStatus = status }

        SetSSEConnectionState state ->
            { model | sseConnectionState = state }
